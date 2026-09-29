use super::*;

#[derive(sqlx::FromRow)]
pub(super) struct DispatchRevisionRow {
    pub(super) release_agent_id: Uuid,
    pub(super) release_id: Uuid,
    pub(super) parameter_hash: Vec<u8>,
    pub(super) platform_policy_version: String,
    pub(super) repository_id: Option<Uuid>,
}

#[derive(sqlx::FromRow)]
pub(super) struct DispatchBindingRow {
    pub(super) binding_id: Uuid,
    pub(super) slot_key: String,
    pub(super) delivery_mode: String,
    pub(super) destinations: Vec<String>,
    pub(super) effective_policy_hash: Vec<u8>,
    pub(super) import_id: Uuid,
    pub(super) grant_id: Uuid,
    pub(super) secret_id: Uuid,
    pub(super) owner_organization_id: Uuid,
    pub(super) version_id: Uuid,
}

impl<K: KeyProvider + Send + Sync> SecretService<K> {
    /// Pins every live binding to an exact immutable version and issues one
    /// short-lived runtime credential immediately before guest dispatch.
    ///
    /// A retry never returns or replaces a previously minted bearer token.
    /// The caller must abandon the first run/session and create a new logical
    /// dispatch if the one-time response was lost.
    ///
    /// # Errors
    ///
    /// Fails closed when exact run, attachment, instance, release, revision,
    /// binding, import, grant, secret, or version authority is stale; when the
    /// actor cannot execute the attachment or use the release; or when a token
    /// was already issued for this command or run.
    #[allow(clippy::too_many_lines)]
    #[tracing::instrument(
          skip_all,
          fields(
              actor_id = %identity.user_id,
              request_id = %identity.request_id,
              run_id = %command.run_id,
              instance_id = %command.instance_id,
              revision_id = %command.instance_revision_id
          )
      )]
    pub async fn resolve_for_dispatch(
        &self,
        identity: &AuthenticatedIdentity,
        command: ResolveRunSecrets,
    ) -> Result<RuntimeSecretAuthority, SecretServiceError> {
        let now = OffsetDateTime::now_utc();
        let lifetime = command.expires_at - now;
        if lifetime <= time::Duration::ZERO || lifetime > time::Duration::minutes(15) {
            return Err(SecretServiceError::InvalidLeaseLifetime);
        }
        match (
            command.phase,
            command.attachment_id,
            command.target_ref.as_ref(),
            command.target_commit.as_ref(),
        ) {
            (ExecutionPhase::Normal, Some(_), Some(_), Some(_))
            | (ExecutionPhase::Update, None, None, None) => {}
            _ => return Err(SecretServiceError::Unavailable),
        }
        let mut tx = begin_actor_transaction(&self.pool, identity)
            .await
            .map_err(|_| SecretServiceError::Persistence)?;
        if let Some(attachment_id) = command.attachment_id {
            self.require(
                &mut tx,
                identity,
                Permission::CanExecute,
                ObjectRef::new(ObjectType::AgentAttachment, attachment_id.as_uuid()),
            )
            .await?;
        } else {
            self.require(
                &mut tx,
                identity,
                Permission::CanUpdate,
                ObjectRef::new(ObjectType::AgentInstance, command.instance_id.as_uuid()),
            )
            .await?;
        }
        if existing_command(&mut tx, command.command_key, "resolve")
            .await
            .map_err(|_| SecretServiceError::Persistence)?
            .is_some()
        {
            return Err(SecretServiceError::CredentialAlreadyIssued);
        }
        let exact: DispatchRevisionRow = match command.phase {
            ExecutionPhase::Normal => {
                let attachment_id = command
                    .attachment_id
                    .ok_or(SecretServiceError::Unavailable)?;
                let target_ref = command
                    .target_ref
                    .as_ref()
                    .ok_or(SecretServiceError::Unavailable)?;
                sqlx::query_as(
                    "SELECT revision.release_agent_id, release.id AS release_id,
                              revision.parameter_hash,
                              revision.platform_policy_version,
                              CASE WHEN revision.publication_mode = 'runtime_git'
                                   THEN CASE WHEN git_binding.binding_id IS NOT NULL
                                             THEN publication_binding.resource_id
                                        END
                                   ELSE attachment.repository_id
                              END AS repository_id
                       FROM runs AS execution
                       JOIN agent_instances AS instance
                         ON instance.id = execution.instance_id
                        AND instance.id = $2
                       JOIN agent_instance_revisions AS revision
                         ON revision.id = execution.instance_revision_id
                        AND revision.id = $3
                        AND revision.instance_id = instance.id
                       JOIN release_agents AS release_agent
                         ON release_agent.id = execution.release_agent_id
                        AND release_agent.id = revision.release_agent_id
                       JOIN releases AS release
                         ON release.id = execution.release_id
                        AND release.id = release_agent.release_id
                       LEFT JOIN agent_capability_bindings AS publication_binding
                         ON publication_binding.id = revision.publication_repository_binding_id
                        AND publication_binding.instance_revision_id = revision.id
                        AND publication_binding.resource_kind = 'repository'
                       LEFT JOIN agent_git_capability_bindings AS git_binding
                         ON git_binding.binding_id = publication_binding.id
                        AND git_binding.instance_revision_id = revision.id
                       JOIN agent_attachments AS attachment
                         ON attachment.id = execution.attachment_id
                        AND attachment.id = $4
                        AND attachment.instance_id = instance.id
                       WHERE execution.id = $1
                         AND execution.run_kind = 'normal'
                         AND execution.state IN (
                             'queued', 'leasing_volume', 'provisioning'
                         )
                         AND instance.state IN ('active', 'update_rejected')
                         AND instance.active_revision_id = revision.id
                         AND revision.runnable
                         AND release.state = 'published'
                         AND attachment.enabled
                         AND attachment.removed_at IS NULL
                         AND attachment.ref_selector = $5
                       FOR UPDATE OF execution, instance",
                )
                .bind(command.run_id.as_uuid())
                .bind(command.instance_id.as_uuid())
                .bind(command.instance_revision_id.as_uuid())
                .bind(attachment_id.as_uuid())
                .bind(target_ref.as_str())
                .fetch_optional(&mut *tx)
                .await
                .map_err(|_| SecretServiceError::Persistence)?
            }
            ExecutionPhase::Update => sqlx::query_as(
                "SELECT revision.release_agent_id, release.id AS release_id,
                              revision.parameter_hash,
                              revision.platform_policy_version,
                              NULL::uuid AS repository_id
                       FROM runs AS execution
                       JOIN agent_updates AS update
                         ON update.hook_run_id = execution.id
                        AND update.instance_id = execution.instance_id
                        AND update.candidate_revision_id =
                            execution.instance_revision_id
                       JOIN agent_instances AS instance
                         ON instance.id = execution.instance_id
                        AND instance.id = $2
                       JOIN agent_instance_revisions AS revision
                         ON revision.id = execution.instance_revision_id
                        AND revision.id = $3
                        AND revision.instance_id = instance.id
                       JOIN release_agents AS release_agent
                         ON release_agent.id = execution.release_agent_id
                        AND release_agent.id = revision.release_agent_id
                       JOIN releases AS release
                         ON release.id = execution.release_id
                        AND release.id = release_agent.release_id
                       WHERE execution.id = $1
                         AND execution.run_kind = 'update'
                         AND execution.attachment_id IS NULL
                         AND execution.state IN (
                             'queued', 'leasing_volume', 'provisioning'
                         )
                         AND update.state = 'hook_running'
                         AND instance.state = 'updating'
                         AND NOT instance.run_gate_open
                         AND revision.runnable
                         AND release.state = 'published'
                         AND NOT EXISTS (
                             SELECT 1 FROM run_instance_provenance
                             WHERE run_id = execution.id
                         )
                       FOR UPDATE OF execution, instance",
            )
            .bind(command.run_id.as_uuid())
            .bind(command.instance_id.as_uuid())
            .bind(command.instance_revision_id.as_uuid())
            .fetch_optional(&mut *tx)
            .await
            .map_err(|_| SecretServiceError::Persistence)?,
        }
        .ok_or(SecretServiceError::Unavailable)?;
        self.require(
            &mut tx,
            identity,
            Permission::CanUse,
            ObjectRef::new(ObjectType::ReleaseAgent, exact.release_agent_id),
        )
        .await?;

        let bindings: Vec<DispatchBindingRow> = sqlx::query_as(
            "SELECT binding.id AS binding_id, binding.slot_key,
                      binding.delivery_mode, binding.destinations,
                      binding.effective_policy_hash,
                      imported.id AS import_id, source_grant.id AS grant_id,
                      secret.id AS secret_id,
                      secret.owner_organization_id,
                      version.id AS version_id
               FROM agent_secret_bindings AS binding
               JOIN secret_imports AS imported
                 ON imported.id = binding.import_id
               JOIN secret_grants AS source_grant
                 ON source_grant.id = imported.grant_id
                AND source_grant.secret_id = imported.secret_id
               JOIN secrets AS secret ON secret.id = imported.secret_id
               JOIN secret_versions AS version
                 ON version.id = secret.active_version_id
                AND version.secret_id = secret.id
               JOIN agent_instances AS instance ON instance.id = $3
               LEFT JOIN agent_attachments AS attachment
                 ON attachment.id = $4 AND attachment.instance_id = instance.id
               WHERE binding.instance_revision_id = $1
                 AND binding.status = 'active'
                 AND imported.status = 'active'
                 AND source_grant.status = 'active'
                 AND secret.status = 'active'
                 AND version.status = 'active'
                 AND $2 = ANY(binding.phases)
                 AND $2 = ANY(source_grant.phases)
                 AND (
                     (
                         $2 = 'normal'
                         AND $4 IS NOT NULL
                         AND attachment.enabled
                         AND attachment.removed_at IS NULL
                         AND $4 = ANY(binding.attachment_ids)
                     )
                     OR (
                         $2 = 'update'
                         AND $4 IS NULL
                         AND imported.target_kind = 'project'
                         AND imported.target_id = instance.project_id
                     )
                 )
                 AND binding.delivery_mode = ANY(source_grant.delivery_modes)
                 AND binding.delivery_mode = ANY(secret.allowed_delivery_modes)
                 AND (
                     source_grant.expires_at IS NULL
                     OR source_grant.expires_at > now()
                 )
                 AND (
                     cardinality(source_grant.destinations) = 0
                     OR source_grant.destinations @> binding.destinations
                 )
                 AND (
                     (imported.target_kind = 'project'
                      AND imported.target_id = instance.project_id)
                     OR
                     (imported.target_kind = 'repository'
                      AND imported.target_id = attachment.repository_id
                      AND $2 = 'normal')
                 )
                 AND imported.target_kind = source_grant.target_kind
                 AND imported.target_id = source_grant.target_id
               ORDER BY binding.slot_key",
        )
        .bind(command.instance_revision_id.as_uuid())
        .bind(phase_name(command.phase))
        .bind(command.instance_id.as_uuid())
        .bind(command.attachment_id.map(AgentAttachmentId::as_uuid))
        .fetch_all(&mut *tx)
        .await
        .map_err(|_| SecretServiceError::Persistence)?;
        let mut expected: Vec<Uuid> = sqlx::query_scalar(
            "SELECT id FROM agent_secret_bindings
               WHERE instance_revision_id = $1 AND $2 = ANY(phases)",
        )
        .bind(command.instance_revision_id.as_uuid())
        .bind(phase_name(command.phase))
        .fetch_all(&mut *tx)
        .await
        .map_err(|_| SecretServiceError::Persistence)?;
        let mut resolved = bindings
            .iter()
            .map(|binding| binding.binding_id)
            .collect::<Vec<_>>();
        expected.sort_unstable();
        resolved.sort_unstable();
        if expected != resolved {
            return Err(SecretServiceError::Unavailable);
        }

        let (credential, leases) = self
            .issue_dispatch_leases(&mut tx, identity, &command, exact, bindings)
            .await?;
        tx.commit()
            .await
            .map_err(|_| SecretServiceError::Persistence)?;
        Ok(RuntimeSecretAuthority {
            session_id: command.session_id,
            credential,
            leases,
        })
    }
}
