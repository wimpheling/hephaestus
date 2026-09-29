use super::*;

#[derive(sqlx::FromRow)]
pub(super) struct RuntimeSessionRow {
    pub(super) session_id: Uuid,
    pub(super) run_id: Uuid,
    pub(super) instance_id: Uuid,
    pub(super) instance_revision_id: Uuid,
    pub(super) attachment_id: Option<Uuid>,
    pub(super) phase: String,
    pub(super) expires_at: OffsetDateTime,
}

#[derive(sqlx::FromRow)]
pub(super) struct BrokeredSecretDenialContextRow {
    pub(super) session_id: Uuid,
    pub(super) run_id: Uuid,
    pub(super) instance_id: Uuid,
    pub(super) instance_revision_id: Uuid,
    pub(super) attachment_id: Option<Uuid>,
    pub(super) phase: String,
    pub(super) expires_at: OffsetDateTime,
    pub(super) lease_id: Uuid,
    pub(super) secret_version_id: Uuid,
    pub(super) destinations: Vec<String>,
}

#[derive(sqlx::FromRow)]
pub(super) struct RuntimeLeaseAuthorizationRow {
    pub(super) lease_id: Uuid,
    pub(super) secret_version_id: Uuid,
    pub(super) destinations: Vec<String>,
}

#[derive(sqlx::FromRow)]
pub(super) struct RuntimeEncryptedVersionRow {
    pub(super) secret_id: Uuid,
    pub(super) version_id: Uuid,
    pub(super) sequence: i64,
    pub(super) organization_id: Option<Uuid>,
    pub(super) project_id: Option<Uuid>,
    pub(super) algorithm: String,
    pub(super) key_reference: String,
    pub(super) data_nonce: Vec<u8>,
    pub(super) ciphertext: Vec<u8>,
    pub(super) wrap_nonce: Vec<u8>,
    pub(super) wrapped_data_key: Vec<u8>,
    pub(super) associated_data_hash: Vec<u8>,
    pub(super) content_length: i32,
}

#[derive(sqlx::FromRow)]
pub(super) struct GatewayEncryptedVersionRow {
    pub(super) header_name: String,
    pub(super) secret_id: Uuid,
    pub(super) version_id: Uuid,
    pub(super) sequence: i64,
    pub(super) organization_id: Option<Uuid>,
    pub(super) project_id: Option<Uuid>,
    pub(super) algorithm: String,
    pub(super) key_reference: String,
    pub(super) data_nonce: Vec<u8>,
    pub(super) ciphertext: Vec<u8>,
    pub(super) wrap_nonce: Vec<u8>,
    pub(super) wrapped_data_key: Vec<u8>,
    pub(super) associated_data_hash: Vec<u8>,
    pub(super) content_length: i32,
}

pub(super) async fn load_runtime_version(
    resolver_pool: &PgPool,
    session: &RuntimeSessionRow,
    lease: &RuntimeLeaseAuthorizationRow,
    mode: DeliveryMode,
    raw_observed: bool,
    outcome: &str,
) -> Result<(VersionContext, EncryptedSecretVersion), SecretServiceError> {
    let mut tx = resolver_pool
        .begin()
        .await
        .map_err(|_| SecretServiceError::Persistence)?;
    let row: RuntimeEncryptedVersionRow = sqlx::query_as(
        "SELECT secret.id AS secret_id, version.id AS version_id,
                  version.sequence, secret.organization_id, secret.project_id,
                  version.algorithm, version.key_reference, version.data_nonce,
                  version.ciphertext, version.wrap_nonce,
                  version.wrapped_data_key, version.associated_data_hash,
                  version.content_length
           FROM secret_leases AS lease
           JOIN secret_runtime_sessions AS session
             ON session.id = lease.session_id AND session.run_id = lease.run_id
           JOIN run_instance_provenance AS run_provenance
             ON run_provenance.run_id = lease.run_id
           JOIN run_secret_provenance AS secret_provenance
             ON secret_provenance.run_id = lease.run_id
            AND secret_provenance.binding_id = lease.binding_id
            AND secret_provenance.secret_version_id = lease.secret_version_id
           JOIN agent_secret_bindings AS binding
             ON binding.id = lease.binding_id
           JOIN secret_imports AS imported
             ON imported.id = secret_provenance.import_id
            AND imported.id = binding.import_id
           JOIN secret_grants AS source_grant
             ON source_grant.id = secret_provenance.grant_id
            AND source_grant.id = imported.grant_id
           JOIN secrets AS secret
             ON secret.id = secret_provenance.secret_id
            AND secret.id = imported.secret_id
           JOIN secret_versions AS version
             ON version.id = lease.secret_version_id
            AND version.secret_id = secret.id
           WHERE lease.id = $1 AND lease.secret_version_id = $2
             AND lease.session_id = $3 AND lease.run_id = $4
             AND lease.delivery_mode = $5
             AND lease.status = 'active' AND lease.expires_at > now()
             AND session.status = 'active' AND session.expires_at > now()
             AND session.instance_id = $6
             AND session.instance_revision_id = $7
             AND session.attachment_id IS NOT DISTINCT FROM $8
             AND session.phase = $9
             AND run_provenance.instance_id = session.instance_id
             AND run_provenance.instance_revision_id = session.instance_revision_id
             AND run_provenance.attachment_id
                 IS NOT DISTINCT FROM session.attachment_id
             AND run_provenance.phase = session.phase
             AND binding.status = 'active'
             AND imported.status = 'active'
             AND source_grant.status = 'active'
             AND secret.status = 'active'
             AND version.status = 'active'
             AND (source_grant.expires_at IS NULL
                  OR source_grant.expires_at > now())",
    )
    .bind(lease.lease_id)
    .bind(lease.secret_version_id)
    .bind(session.session_id)
    .bind(session.run_id)
    .bind(mode_name(mode))
    .bind(session.instance_id)
    .bind(session.instance_revision_id)
    .bind(session.attachment_id)
    .bind(&session.phase)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|_| SecretServiceError::Persistence)?
    .ok_or(SecretServiceError::Unavailable)?;
    if raw_observed {
        sqlx::query(
            "UPDATE secret_leases
               SET raw_material_observed = true
               WHERE id = $1 AND status = 'active'",
        )
        .bind(lease.lease_id)
        .execute(&mut *tx)
        .await
        .map_err(|_| SecretServiceError::Persistence)?;
    }
    record_runtime_use_tx(&mut tx, session, lease, mode, outcome)
        .await
        .map_err(|_| SecretServiceError::Persistence)?;
    tx.commit()
        .await
        .map_err(|_| SecretServiceError::Persistence)?;
    encrypted_version(row)
}

pub(super) fn encrypted_version(
    row: RuntimeEncryptedVersionRow,
) -> Result<(VersionContext, EncryptedSecretVersion), SecretServiceError> {
    let owner = match (row.organization_id, row.project_id) {
        (Some(id), None) => SecretOwner::Organization(OrganizationId::from_uuid(id)),
        (None, Some(id)) => SecretOwner::Project(ProjectId::from_uuid(id)),
        _ => return Err(SecretServiceError::InvalidStoredData),
    };
    let version_id = SecretVersionId::from_uuid(row.version_id);
    let context = VersionContext {
        owner,
        secret_id: SecretId::from_uuid(row.secret_id),
        version_id,
        sequence: u64::try_from(row.sequence).map_err(|_| SecretServiceError::InvalidStoredData)?,
        media_type: String::from("application/octet-stream"),
    };
    let encrypted = EncryptedSecretVersion {
        version_id,
        algorithm: row.algorithm,
        key_reference: row.key_reference,
        data_nonce: row
            .data_nonce
            .try_into()
            .map_err(|_| SecretServiceError::InvalidStoredData)?,
        ciphertext: row.ciphertext,
        wrap_nonce: row
            .wrap_nonce
            .try_into()
            .map_err(|_| SecretServiceError::InvalidStoredData)?,
        wrapped_data_key: row.wrapped_data_key,
        associated_data_hash: row
            .associated_data_hash
            .try_into()
            .map_err(|_| SecretServiceError::InvalidStoredData)?,
        content_length: u32::try_from(row.content_length)
            .map_err(|_| SecretServiceError::InvalidStoredData)?,
    };
    Ok((context, encrypted))
}

pub(super) fn gateway_encrypted_version(
    row: GatewayEncryptedVersionRow,
) -> Result<(VersionContext, EncryptedSecretVersion), GatewayError> {
    let owner = match (row.organization_id, row.project_id) {
        (Some(id), None) => SecretOwner::Organization(OrganizationId::from_uuid(id)),
        (None, Some(id)) => SecretOwner::Project(ProjectId::from_uuid(id)),
        _ => return Err(GatewayError::InvalidInboundSecretRule),
    };
    let version_id = SecretVersionId::from_uuid(row.version_id);
    let context = VersionContext {
        owner,
        secret_id: SecretId::from_uuid(row.secret_id),
        version_id,
        sequence: u64::try_from(row.sequence)
            .map_err(|_| GatewayError::InvalidInboundSecretRule)?,
        media_type: String::from("application/octet-stream"),
    };
    let encrypted = EncryptedSecretVersion {
        version_id,
        algorithm: row.algorithm,
        key_reference: row.key_reference,
        data_nonce: row
            .data_nonce
            .try_into()
            .map_err(|_| GatewayError::InvalidInboundSecretRule)?,
        ciphertext: row.ciphertext,
        wrap_nonce: row
            .wrap_nonce
            .try_into()
            .map_err(|_| GatewayError::InvalidInboundSecretRule)?,
        wrapped_data_key: row.wrapped_data_key,
        associated_data_hash: row
            .associated_data_hash
            .try_into()
            .map_err(|_| GatewayError::InvalidInboundSecretRule)?,
        content_length: u32::try_from(row.content_length)
            .map_err(|_| GatewayError::InvalidInboundSecretRule)?,
    };
    Ok((context, encrypted))
}
