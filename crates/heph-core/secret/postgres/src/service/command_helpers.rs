use super::*;

pub(super) fn parse_mode(value: &str) -> Result<DeliveryMode, SecretServiceError> {
    match value {
        "raw" => Ok(DeliveryMode::Raw),
        "brokered" => Ok(DeliveryMode::Brokered),
        _ => Err(SecretServiceError::InvalidStoredData),
    }
}

#[derive(sqlx::FromRow)]
pub(super) struct SecretRotationRow {
    pub(super) owner_organization_id: Uuid,
    pub(super) organization_id: Option<Uuid>,
    pub(super) project_id: Option<Uuid>,
    pub(super) active_version_id: Option<Uuid>,
    pub(super) sequence: i64,
}

impl SecretRotationRow {
    pub(super) const fn owner(&self) -> Result<SecretOwner, SecretServiceError> {
        match (self.organization_id, self.project_id) {
            (Some(id), None) => Ok(SecretOwner::Organization(OrganizationId::from_uuid(id))),
            (None, Some(id)) => Ok(SecretOwner::Project(ProjectId::from_uuid(id))),
            _ => Err(SecretServiceError::InvalidStoredData),
        }
    }
}

#[derive(sqlx::FromRow)]
pub(super) struct GrantAcceptanceRow {
    pub(super) secret_id: Uuid,
    pub(super) owner_organization_id: Uuid,
    pub(super) target_kind: String,
    pub(super) target_id: Uuid,
}

pub(super) struct ResolvedTarget {
    pub(super) kind: &'static str,
    pub(super) object_type: ObjectType,
    pub(super) id: Uuid,
    pub(super) project_id: Uuid,
    pub(super) organization_id: Uuid,
}

pub(super) async fn resolve_target(
    tx: &mut Transaction<'_, Postgres>,
    target: SecretTarget,
) -> Result<ResolvedTarget, SecretServiceError> {
    match target {
        SecretTarget::Project(id) => {
            let organization_id: Uuid =
                sqlx::query_scalar("SELECT organization_id FROM projects WHERE id = $1")
                    .bind(id.as_uuid())
                    .fetch_optional(&mut **tx)
                    .await
                    .map_err(|_| SecretServiceError::Persistence)?
                    .ok_or(SecretServiceError::Unavailable)?;
            Ok(ResolvedTarget {
                kind: "project",
                object_type: ObjectType::Project,
                id: id.as_uuid(),
                project_id: id.as_uuid(),
                organization_id,
            })
        }
        SecretTarget::Repository(id) => {
            let row: (Uuid, Uuid) = sqlx::query_as(
                "SELECT repositories.project_id, projects.organization_id
                   FROM repositories
                   JOIN projects ON projects.id = repositories.project_id
                   WHERE repositories.id = $1",
            )
            .bind(id.as_uuid())
            .fetch_optional(&mut **tx)
            .await
            .map_err(|_| SecretServiceError::Persistence)?
            .ok_or(SecretServiceError::Unavailable)?;
            Ok(ResolvedTarget {
                kind: "repository",
                object_type: ObjectType::Repository,
                id: id.as_uuid(),
                project_id: row.0,
                organization_id: row.1,
            })
        }
    }
}

pub(super) async fn resolve_owner(
    tx: &mut Transaction<'_, Postgres>,
    owner: SecretOwner,
) -> Result<(ObjectType, Uuid, Uuid, Option<Uuid>, Option<Uuid>), SecretServiceError> {
    match owner {
        SecretOwner::Organization(id) => {
            let exists: bool =
                sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM organizations WHERE id = $1)")
                    .bind(id.as_uuid())
                    .fetch_one(&mut **tx)
                    .await
                    .map_err(|_| SecretServiceError::Persistence)?;
            if !exists {
                return Err(SecretServiceError::Unavailable);
            }
            Ok((
                ObjectType::Organization,
                id.as_uuid(),
                id.as_uuid(),
                None,
                Some(id.as_uuid()),
            ))
        }
        SecretOwner::Project(id) => {
            let organization_id: Uuid =
                sqlx::query_scalar("SELECT organization_id FROM projects WHERE id = $1")
                    .bind(id.as_uuid())
                    .fetch_optional(&mut **tx)
                    .await
                    .map_err(|_| SecretServiceError::Persistence)?
                    .ok_or(SecretServiceError::Unavailable)?;
            Ok((
                ObjectType::Project,
                id.as_uuid(),
                organization_id,
                Some(id.as_uuid()),
                None,
            ))
        }
    }
}

pub(super) fn normalized_modes(
    modes: &[DeliveryMode],
) -> Result<Vec<&'static str>, SecretServiceError> {
    let mut names = modes.iter().copied().map(mode_name).collect::<Vec<_>>();
    names.sort_unstable();
    names.dedup();
    if names.is_empty() {
        return Err(SecretServiceError::InvalidDeliveryModes);
    }
    Ok(names)
}

pub(super) const fn mode_name(mode: DeliveryMode) -> &'static str {
    match mode {
        DeliveryMode::Raw => "raw",
        DeliveryMode::Brokered => "brokered",
    }
}

pub(super) const fn phase_name(phase: ExecutionPhase) -> &'static str {
    match phase {
        ExecutionPhase::Normal => "normal",
        ExecutionPhase::Update => "update",
    }
}

pub(super) async fn insert_encrypted_version(
    tx: &mut Transaction<'_, Postgres>,
    secret_id: SecretId,
    sequence: u64,
    encrypted: &EncryptedSecretVersion,
    creator_id: Uuid,
) -> Result<(), SecretServiceError> {
    let sequence =
        i64::try_from(sequence).map_err(|_| SecretServiceError::VersionSequenceExhausted)?;
    sqlx::query(
        "INSERT INTO secret_versions
           (id, secret_id, sequence, status, algorithm, key_reference,
            data_nonce, ciphertext, wrap_nonce, wrapped_data_key,
            associated_data_hash, content_length, created_by)
           VALUES ($1, $2, $3, 'active', $4, $5, $6, $7, $8, $9, $10, $11, $12)",
    )
    .bind(encrypted.version_id.as_uuid())
    .bind(secret_id.as_uuid())
    .bind(sequence)
    .bind(&encrypted.algorithm)
    .bind(&encrypted.key_reference)
    .bind(encrypted.data_nonce.as_slice())
    .bind(&encrypted.ciphertext)
    .bind(encrypted.wrap_nonce.as_slice())
    .bind(&encrypted.wrapped_data_key)
    .bind(encrypted.associated_data_hash.as_slice())
    .bind(i32::try_from(encrypted.content_length).unwrap_or(i32::MAX))
    .bind(creator_id)
    .execute(&mut **tx)
    .await
    .map_err(|_| SecretServiceError::Persistence)?;
    Ok(())
}

pub(super) async fn existing_command(
    tx: &mut Transaction<'_, Postgres>,
    command_key: SecretCommandKey,
    operation: &str,
) -> Result<Option<(Uuid, Option<Uuid>)>, SecretServiceError> {
    let row: Option<(String, Uuid, Option<Uuid>)> = sqlx::query_as(
        "SELECT operation, aggregate_id, secondary_id
           FROM secret_command_inbox WHERE command_key = $1",
    )
    .bind(command_key.as_bytes().as_slice())
    .fetch_optional(&mut **tx)
    .await
    .map_err(|_| SecretServiceError::Persistence)?;
    match row {
        Some((stored_operation, aggregate_id, secondary_id)) if stored_operation == operation => {
            Ok(Some((aggregate_id, secondary_id)))
        }
        Some(_) => Err(SecretServiceError::IdempotencyConflict),
        None => Ok(None),
    }
}

pub(super) async fn record_command(
    tx: &mut Transaction<'_, Postgres>,
    command_key: SecretCommandKey,
    operation: &str,
    aggregate_id: Uuid,
    secondary_id: Option<Uuid>,
    identity: &AuthenticatedIdentity,
) -> Result<(), SecretServiceError> {
    sqlx::query(
        "INSERT INTO secret_command_inbox
           (command_key, operation, aggregate_id, secondary_id,
            requester_id, request_id)
           VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(command_key.as_bytes().as_slice())
    .bind(operation)
    .bind(aggregate_id)
    .bind(secondary_id)
    .bind(identity.user_id.as_uuid())
    .bind(identity.request_id.as_uuid())
    .execute(&mut **tx)
    .await
    .map_err(|_| SecretServiceError::Persistence)?;
    Ok(())
}

// Audit fields stay explicit so values cannot be hidden in an untyped payload.
#[allow(clippy::too_many_arguments)]
pub(super) async fn audit(
    tx: &mut Transaction<'_, Postgres>,
    identity: &AuthenticatedIdentity,
    owner_organization_id: Uuid,
    operation: &str,
    permission: &str,
    secret_id: Option<SecretId>,
    version_id: Option<SecretVersionId>,
    grant_id: Option<SecretGrantId>,
    import_id: Option<SecretImportId>,
    outcome: &str,
) -> Result<(), SecretServiceError> {
    sqlx::query(
        "INSERT INTO secret_audit_events
           (id, owner_organization_id, requester_id, secret_id,
            secret_version_id, grant_id, import_id, operation, permission,
            target_kind, target_id, delivery_mode, decision, outcome,
            request_id, command_id, authorization_model_version, policy_version)
           SELECT $1, $2, $3, $4, $5, $6, $7, $8, $9,
                  COALESCE(secret_import.target_kind, secret_grant.target_kind),
                  COALESCE(secret_import.target_id, secret_grant.target_id),
                  CASE
                      WHEN $8 IN ('bind_raw', 'receive_raw') THEN 'raw'
                      WHEN $8 IN ('bind_brokered', 'use_brokered') THEN 'brokered'
                      ELSE NULL
                  END,
                  'allow', $10, $11, $11, $12, 'command/v1'
           FROM (SELECT 1) AS singleton
           LEFT JOIN secret_grants AS secret_grant ON secret_grant.id = $6
           LEFT JOIN secret_imports AS secret_import ON secret_import.id = $7",
    )
    .bind(Uuid::new_v4())
    .bind(owner_organization_id)
    .bind(identity.user_id.as_uuid())
    .bind(secret_id.map(SecretId::as_uuid))
    .bind(version_id.map(SecretVersionId::as_uuid))
    .bind(grant_id.map(SecretGrantId::as_uuid))
    .bind(import_id.map(SecretImportId::as_uuid))
    .bind(operation)
    .bind(permission)
    .bind(outcome)
    .bind(identity.request_id.as_uuid())
    .bind(AUTHORIZATION_MODEL_VERSION)
    .execute(&mut **tx)
    .await
    .map_err(|_| SecretServiceError::Persistence)?;
    Ok(())
}
