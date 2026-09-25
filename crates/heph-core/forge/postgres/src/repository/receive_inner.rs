use super::{
    PgForgeRepository,
    builds::{build_trigger_matches, persist_build_request},
    helpers::storage,
    inspect::inspect_updates,
    replay::replay_receive,
    revisions::{persist_repository_oci_image_revisions, persist_revision},
    rows::{ExistingReceiveProvenance, RunRequestRow},
    triggers::persist_instance_triggers,
};
use crate::repository::{ui_manifest, ui_manifest_store};
use agent_config::REUSABLE_RELEASE_VERSION;
use authz_domain::{AuthorizationDecision, ObjectRef, ObjectType, Permission, Subject};
use authz_postgres::{audit_decision, begin_actor_transaction};
use forge_domain::{CommitSha, ReceiveId, RefUpdate, Repository, RuntimeReceiveProvenance};
use forge_service::{ForgeRepositoryError, ReceiveResult, RunRequest};
use identity_domain::AuthenticatedIdentity;
use time::OffsetDateTime;
use uuid::Uuid;

impl PgForgeRepository {
    // Keeping this transactional workflow together makes the all-or-nothing
    // receive invariant directly auditable.
    #[allow(clippy::too_many_lines)]
    // The receive transaction deliberately keeps authorization, ref
    // validation, persistence, and outbox publication in one auditable path.
    #[allow(clippy::cognitive_complexity)]
    /// Executes the receive persistence transaction.
    ///
    /// # Errors
    ///
    /// Returns a repository error when inspection, authorization, or persistence fails.
    pub async fn accept_receive_inner(
        &self,
        repository: &Repository,
        receive_id: ReceiveId,
        principal: &str,
        identity: Option<&AuthenticatedIdentity>,
        updates: &[RefUpdate],
        runtime_provenance: Option<RuntimeReceiveProvenance>,
    ) -> Result<ReceiveResult, ForgeRepositoryError> {
        if identity.is_some() && self.authorizer.is_none() {
            return Err(ForgeRepositoryError::AuthorizationUnavailable);
        }
        let mut transaction = match identity {
            Some(identity) => begin_actor_transaction(&self.pool, identity)
                .await
                .map_err(storage)?,
            None => self.pool.begin().await.map_err(storage)?,
        };
        let runtime_attachment_id = if let Some(provenance) = runtime_provenance {
            let resolved = sqlx::query_as::<_, (Option<Uuid>,)>(
                "SELECT * FROM resolve_runtime_receive_attachment($1, $2)",
            )
            .bind(provenance.runtime_session_id)
            .bind(repository.id.as_uuid())
            .fetch_optional(&mut *transaction)
            .await
            .map_err(storage)?;
            let Some((attachment,)) = resolved else {
                return Err(ForgeRepositoryError::InvalidStoredData(
                    "runtime receive provenance",
                ));
            };
            attachment
        } else {
            None
        };
        let now = OffsetDateTime::now_utc();
        let inserted = sqlx::query(
            "INSERT INTO git_receives
             (id, repository_id, actor_id, principal, request_id,
              runtime_session_id, runtime_attachment_id,
              status, accepted_at, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, 'accepted', $8, $8)
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(receive_id.as_uuid())
        .bind(repository.id.as_uuid())
        .bind(identity.map(|value| value.user_id.as_uuid()))
        .bind(principal)
        .bind(identity.map(|value| value.request_id.as_uuid()))
        .bind(runtime_provenance.map(|value| value.runtime_session_id))
        .bind(runtime_attachment_id)
        .bind(now)
        .execute(&mut *transaction)
        .await
        .map_err(storage)?;
        if inserted.rows_affected() == 0 {
            let existing = sqlx::query_as::<_, ExistingReceiveProvenance>(
                "SELECT repository_id AS repository,
                        runtime_session_id AS runtime_session,
                        runtime_attachment_id AS runtime_attachment
                 FROM git_receives WHERE id = $1",
            )
            .bind(receive_id.as_uuid())
            .fetch_one(&mut *transaction)
            .await
            .map_err(storage)?;
            return replay_receive(
                transaction,
                receive_id,
                repository.id,
                runtime_provenance.map(|value| value.runtime_session_id),
                runtime_attachment_id,
                existing,
            )
            .await;
        }
        sqlx::query("SELECT id FROM repositories WHERE id = $1 FOR NO KEY UPDATE")
            .bind(repository.id.as_uuid())
            .fetch_one(&mut *transaction)
            .await
            .map_err(storage)?;
        let repository_path = self.storage.validate_existing(repository.id).await?;
        let inspected = inspect_updates(&repository_path, updates)?;
        for (index, update) in updates.iter().enumerate() {
            let sequence = i32::try_from(index + 1)
                .map_err(|_| ForgeRepositoryError::InvalidMetadata("too many ref updates"))?;
            sqlx::query(
                "INSERT INTO git_ref_updates
                 (receive_id, sequence, git_ref, old_commit, new_commit)
                 VALUES ($1, $2, $3, $4, $5)
                 ON CONFLICT (receive_id, sequence) DO NOTHING",
            )
            .bind(receive_id.as_uuid())
            .bind(sequence)
            .bind(update.git_ref.as_str())
            .bind(update.old_commit.as_ref().map(CommitSha::as_str))
            .bind(update.new_commit.as_ref().map(CommitSha::as_str))
            .execute(&mut *transaction)
            .await
            .map_err(storage)?;
            if let Some(new_commit) = &update.new_commit {
                sqlx::query(
                    "INSERT INTO git_refs
                     (repository_id, git_ref, commit_sha, updated_by_receive_id, updated_at)
                     VALUES ($1, $2, $3, $4, $5)
                     ON CONFLICT (repository_id, git_ref) DO UPDATE
                     SET commit_sha = EXCLUDED.commit_sha,
                         updated_by_receive_id = EXCLUDED.updated_by_receive_id,
                         updated_at = EXCLUDED.updated_at",
                )
                .bind(repository.id.as_uuid())
                .bind(update.git_ref.as_str())
                .bind(new_commit.as_str())
                .bind(receive_id.as_uuid())
                .bind(now)
                .execute(&mut *transaction)
                .await
                .map_err(storage)?;
            } else {
                sqlx::query("DELETE FROM git_refs WHERE repository_id = $1 AND git_ref = $2")
                    .bind(repository.id.as_uuid())
                    .bind(update.git_ref.as_str())
                    .execute(&mut *transaction)
                    .await
                    .map_err(storage)?;
            }
        }
        let mut build_requests = Vec::new();
        let mut invalid_configurations = 0;
        for item in inspected {
            if let Some(images) = item.repository_oci_images.as_deref() {
                persist_repository_oci_image_revisions(
                    &mut transaction,
                    repository.id,
                    repository.project_id,
                    &item.commit,
                    images,
                )
                .await?;
            }
            let ui_revision = if let Some(inspection) = item.ui_manifest.as_ref() {
                Some(
                    ui_manifest_store::persist_ui_manifest_revision(
                        &mut transaction,
                        repository.id,
                        receive_id,
                        &item.commit,
                        inspection,
                    )
                    .await?,
                )
            } else {
                None
            };
            let Some(parsed) = item.parsed else {
                continue;
            };
            if parsed.config.is_some() {
                if let (Some(identity), Some(authorizer)) = (identity, &self.authorizer) {
                    let object =
                        ObjectRef::new(ObjectType::Project, repository.project_id.as_uuid());
                    let decision = authorizer
                        .check(
                            &mut transaction,
                            Subject::User(identity.user_id),
                            Permission::CanWrite,
                            object,
                        )
                        .await
                        .map_err(storage)?;
                    audit_decision(
                        &mut transaction,
                        identity.user_id,
                        Permission::CanWrite,
                        object,
                        decision,
                        identity.request_id,
                    )
                    .await
                    .map_err(storage)?;
                    if decision == AuthorizationDecision::Deny {
                        return Err(ForgeRepositoryError::AuthorizationDenied);
                    }
                }
            }
            persist_revision(
                &mut transaction,
                repository.id,
                receive_id,
                &item.commit,
                &parsed,
                now,
            )
            .await?;
            let Some(config) = parsed.config.as_ref() else {
                invalid_configurations += 1;
                continue;
            };
            if config.version != REUSABLE_RELEASE_VERSION {
                return Err(ForgeRepositoryError::InvalidStoredData(
                    "unsupported configuration accepted by parser",
                ));
            }
            let build = config
                .build
                .as_ref()
                .ok_or(ForgeRepositoryError::InvalidStoredData(
                    "valid reusable configuration build definition",
                ))?;
            if build_trigger_matches(&build.triggers, &item.git_ref) {
                let base_build_definition_hash =
                    agent_config::build_identity::base_build_definition_hash(build)
                        .map_err(ForgeRepositoryError::Serialization)?;
                let build_definition_hash = if let Some(revision) = ui_revision.as_ref() {
                    if !matches!(revision.status, ui_manifest::UiManifestStatus::Valid) {
                        continue;
                    }
                    let ui_hash = revision.normalized_ui_hash.ok_or(
                        ForgeRepositoryError::InvalidStoredData(
                            "valid repository UI normalized hash",
                        ),
                    )?;
                    agent_config::build_identity::ui_build_definition_hash(
                        base_build_definition_hash,
                        ui_hash,
                        revision.normalized_gateway_hash,
                    )
                } else {
                    base_build_definition_hash
                };
                let build_request_id = persist_build_request(
                    &mut transaction,
                    repository.id,
                    receive_id,
                    &item.git_ref,
                    &item.commit,
                    build,
                    &config.guest.image,
                    config.agent.key.as_deref(),
                    parsed.normalized_hash.as_ref().ok_or(
                        ForgeRepositoryError::InvalidStoredData(
                            "valid reusable configuration normalized hash",
                        ),
                    )?,
                    build_definition_hash,
                    identity,
                    now,
                )
                .await?;
                if let Some(revision) = ui_revision.as_ref() {
                    ui_manifest_store::link_ui_manifest_to_build(
                        &mut transaction,
                        build_request_id,
                        repository.id,
                        &item.commit,
                        revision.id,
                    )
                    .await?;
                }
                build_requests.push(build_request_id);
            }
        }
        persist_instance_triggers(
            &mut transaction,
            repository,
            receive_id,
            identity,
            updates,
            self.authorizer.as_deref(),
            now,
            runtime_attachment_id,
            runtime_provenance.is_some(),
        )
        .await?;
        let rows = sqlx::query_as::<_, RunRequestRow>(
            "SELECT id, repository_id, commit_sha, git_ref, receive_id,
                    instance_id, instance_revision_id, release_id,
                    release_agent_id, attachment_id, run_id, command_id,
                    requires_state
             FROM run_requests
             WHERE receive_id = $1
             ORDER BY created_at, id",
        )
        .bind(receive_id.as_uuid())
        .fetch_all(&mut *transaction)
        .await
        .map_err(storage)?;
        let run_requests = rows
            .into_iter()
            .map(RunRequest::try_from)
            .collect::<Result<Vec<_>, _>>()?;
        transaction.commit().await.map_err(storage)?;
        for update in updates {
            tracing::info!(
                repository_id = %repository.id,
                %receive_id,
                git_ref = %update.git_ref,
                old_commit = update.old_commit.as_ref().map(CommitSha::as_str),
                new_commit = update.new_commit.as_ref().map(CommitSha::as_str),
                principal,
                actor_id = ?identity.map(|value| value.user_id),
                request_id = ?identity.map(|value| value.request_id),
                "accepted Git ref update was persisted"
            );
        }
        Ok(ReceiveResult {
            receive_id,
            run_requests,
            build_requests,
            invalid_configurations,
        })
    }
}
