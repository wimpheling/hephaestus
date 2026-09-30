use super::dispatch_resolution::{DispatchBindingRow, DispatchRevisionRow};
use super::*;

impl<K: KeyProvider + Send + Sync> SecretService<K> {
    #[allow(clippy::too_many_lines)] // Keep the lease and audit writes in one atomic sequence.
    pub(super) async fn issue_dispatch_leases(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        identity: &AuthenticatedIdentity,
        command: &ResolveRunSecrets,
        exact: DispatchRevisionRow,
        bindings: Vec<DispatchBindingRow>,
    ) -> Result<
        (
            secret_domain::OpaqueRuntimeCredential,
            Vec<IssuedSecretLease>,
        ),
        SecretServiceError,
    > {
        let mut credential_bytes = Vec::with_capacity(32);
        credential_bytes.extend_from_slice(Uuid::new_v4().as_bytes());
        credential_bytes.extend_from_slice(Uuid::new_v4().as_bytes());
        let credential = secret_domain::OpaqueRuntimeCredential::new(credential_bytes)?;
        let credential_hash = credential.storage_hash();
        sqlx::query(
            "INSERT INTO run_instance_provenance
               (run_id, instance_id, instance_revision_id, release_id,
                release_agent_id, attachment_id, target_repository_id,
                target_ref, target_commit, parameter_hash,
                platform_policy_version, phase, authorization_model_version)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10,
                       $11, $12, $13)
               ON CONFLICT (run_id) DO NOTHING",
        )
        .bind(command.run_id.as_uuid())
        .bind(command.instance_id.as_uuid())
        .bind(command.instance_revision_id.as_uuid())
        .bind(exact.release_id)
        .bind(exact.release_agent_id)
        .bind(command.attachment_id.map(AgentAttachmentId::as_uuid))
        .bind(exact.repository_id)
        .bind(command.target_ref.as_ref().map(GitRef::as_str))
        .bind(command.target_commit.as_ref().map(CommitSha::as_str))
        .bind(&exact.parameter_hash)
        .bind(&exact.platform_policy_version)
        .bind(phase_name(command.phase))
        .bind(AUTHORIZATION_MODEL_VERSION)
        .execute(&mut **tx)
        .await
        .map_err(|_| SecretServiceError::Persistence)?;
        let provenance_matches: bool = sqlx::query_scalar(
            "SELECT EXISTS (
                 SELECT 1
                 FROM run_instance_provenance
                 WHERE run_id = $1
                   AND instance_id = $2
                   AND instance_revision_id = $3
                   AND release_id = $4
                   AND release_agent_id = $5
                   AND attachment_id IS NOT DISTINCT FROM $6
                   AND target_repository_id IS NOT DISTINCT FROM $7
                   AND target_ref IS NOT DISTINCT FROM $8
                   AND target_commit IS NOT DISTINCT FROM $9
                   AND parameter_hash = $10
                   AND platform_policy_version = $11
                   AND phase = $12
                   AND authorization_model_version = $13
             )",
        )
        .bind(command.run_id.as_uuid())
        .bind(command.instance_id.as_uuid())
        .bind(command.instance_revision_id.as_uuid())
        .bind(exact.release_id)
        .bind(exact.release_agent_id)
        .bind(command.attachment_id.map(AgentAttachmentId::as_uuid))
        .bind(exact.repository_id)
        .bind(command.target_ref.as_ref().map(GitRef::as_str))
        .bind(command.target_commit.as_ref().map(CommitSha::as_str))
        .bind(&exact.parameter_hash)
        .bind(&exact.platform_policy_version)
        .bind(phase_name(command.phase))
        .bind(AUTHORIZATION_MODEL_VERSION)
        .fetch_one(&mut **tx)
        .await
        .map_err(|_| SecretServiceError::Persistence)?;
        if !provenance_matches {
            return Err(SecretServiceError::Unavailable);
        }
        sqlx::query(
            "INSERT INTO secret_runtime_sessions
               (id, run_id, instance_id, instance_revision_id, attachment_id,
                phase, runtime_credential_hash, status, expires_at)
               VALUES ($1, $2, $3, $4, $5, $6, $7, 'active', $8)",
        )
        .bind(command.session_id.as_uuid())
        .bind(command.run_id.as_uuid())
        .bind(command.instance_id.as_uuid())
        .bind(command.instance_revision_id.as_uuid())
        .bind(command.attachment_id.map(AgentAttachmentId::as_uuid))
        .bind(phase_name(command.phase))
        .bind(credential_hash.as_slice())
        .bind(command.expires_at)
        .execute(&mut **tx)
        .await
        .map_err(|_| SecretServiceError::Persistence)?;
        let mut leases = Vec::with_capacity(bindings.len());
        for binding in bindings {
            let lease_id = SecretLeaseId::new();
            let mode = parse_mode(&binding.delivery_mode)?;
            sqlx::query(
                "INSERT INTO run_secret_provenance
                   (run_id, binding_id, secret_id, secret_version_id,
                    grant_id, import_id, authorization_model_version,
                    delivery_policy_hash)
                   VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            )
            .bind(command.run_id.as_uuid())
            .bind(binding.binding_id)
            .bind(binding.secret_id)
            .bind(binding.version_id)
            .bind(binding.grant_id)
            .bind(binding.import_id)
            .bind(AUTHORIZATION_MODEL_VERSION)
            .bind(&binding.effective_policy_hash)
            .execute(&mut **tx)
            .await
            .map_err(|_| SecretServiceError::Persistence)?;
            sqlx::query(
                "INSERT INTO secret_leases
                   (id, session_id, run_id, binding_id, secret_version_id,
                    delivery_mode, slot_key, destinations, status, expires_at)
                   VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'active', $9)",
            )
            .bind(lease_id.as_uuid())
            .bind(command.session_id.as_uuid())
            .bind(command.run_id.as_uuid())
            .bind(binding.binding_id)
            .bind(binding.version_id)
            .bind(&binding.delivery_mode)
            .bind(&binding.slot_key)
            .bind(&binding.destinations)
            .bind(command.expires_at)
            .execute(&mut **tx)
            .await
            .map_err(|_| SecretServiceError::Persistence)?;
            // A brokered HTTPS rule is optional because the legacy semantic
            // broker remains supported. When one exists, its immutable rule
            // and selected version are copied into this exact runtime lease;
            // `https_v1` requests without this snapshot fail closed.
            if binding.delivery_mode == "brokered" {
                sqlx::query(
                    "INSERT INTO brokered_secret_lease_snapshots
                       (id, lease_id, runtime_session_id, run_id, binding_id,
                        secret_version_id, rule_id, destination_origin,
                        location_kind, header_name, header_prefix, rule_hash)
                     SELECT $1, $2, $3, $4, rule.binding_id,
                            rule.secret_version_id, rule.id, rule.destination_origin,
                            rule.location_kind, rule.header_name, rule.header_prefix,
                            rule.normalized_hash
                       FROM brokered_secret_rules AS rule
                       WHERE rule.binding_id = $5
                         AND rule.instance_revision_id = $6
                         AND rule.secret_version_id = $7",
                )
                .bind(Uuid::new_v4())
                .bind(lease_id.as_uuid())
                .bind(command.session_id.as_uuid())
                .bind(command.run_id.as_uuid())
                .bind(binding.binding_id)
                .bind(command.instance_revision_id.as_uuid())
                .bind(binding.version_id)
                .execute(&mut **tx)
                .await
                .map_err(|_| SecretServiceError::Persistence)?;
            }
            audit_resolution(tx, identity, &binding, lease_id, command.run_id)
                .await
                .map_err(|_| SecretServiceError::Persistence)?;
            leases.push(IssuedSecretLease {
                lease_id,
                slot: SecretSlotKey::parse(binding.slot_key)?,
                mode,
                version_id: SecretVersionId::from_uuid(binding.version_id),
            });
        }
        record_command(
            tx,
            command.command_key,
            "resolve",
            command.session_id.as_uuid(),
            Some(command.run_id.as_uuid()),
            identity,
        )
        .await
        .map_err(|_| SecretServiceError::Persistence)?;
        Ok((credential, leases))
    }
}

async fn audit_resolution(
    tx: &mut Transaction<'_, Postgres>,
    identity: &AuthenticatedIdentity,
    binding: &DispatchBindingRow,
    lease_id: SecretLeaseId,
    run_id: RunId,
) -> Result<(), SecretServiceError> {
    sqlx::query(
        "INSERT INTO secret_audit_events
           (id, owner_organization_id, requester_id, runtime_run_id,
            secret_id, secret_version_id, grant_id, import_id, binding_id,
            lease_id, operation, permission, delivery_mode, decision, outcome,
            request_id, authorization_model_version, policy_version)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10,
                   'resolve', $11, $12, 'allow', 'lease_issued',
                   $13, $14, 'dispatch/v1')",
    )
    .bind(Uuid::new_v4())
    .bind(binding.owner_organization_id)
    .bind(identity.user_id.as_uuid())
    .bind(run_id.as_uuid())
    .bind(binding.secret_id)
    .bind(binding.version_id)
    .bind(binding.grant_id)
    .bind(binding.import_id)
    .bind(binding.binding_id)
    .bind(lease_id.as_uuid())
    .bind(if binding.delivery_mode == "raw" {
        "secret.receive_raw"
    } else {
        "secret.use_brokered"
    })
    .bind(&binding.delivery_mode)
    .bind(identity.request_id.as_uuid())
    .bind(AUTHORIZATION_MODEL_VERSION)
    .execute(&mut **tx)
    .await
    .map_err(|_| SecretServiceError::Persistence)?;
    Ok(())
}
