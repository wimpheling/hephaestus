use agent_config::{ConfigHash, ParsedConfig, REUSABLE_RELEASE_VERSION};
use authz_domain::{AuthorizationDecision, ObjectRef, ObjectType, Permission, Subject};
use authz_postgres::{PostgresMelangeAuthorizer, audit_decision, begin_actor_transaction};
use forge_domain::{
    AgentConfigRevisionId, CommitSha, GitRef, OrganizationId, Project, ProjectId, ReceiveId,
    RefUpdate, Repository, RepositoryId, RunRequestId,
};
use forge_service::{
    CreateRepository, ForgeRepositoryError, OutboxRecord, ReceiveResult, RunRequest,
};
use identity_domain::AuthenticatedIdentity;
use release_domain::BuildRequestId;
use run_domain::{RunKind, StartRun};
use runtime_types::{
    AgentAttachmentId, AgentInstanceId, AgentInstanceRevisionId, CommandId, EventId,
    ReleaseAgentId, ReleaseId, RunId,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{FromRow, PgPool, Postgres, Transaction};
use std::{path::Path, sync::Arc};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    BUILD_REQUESTED_SUBJECT, GitStorage, INSTANCE_RUN_REQUESTED_SUBJECT, RUN_START_SUBJECT,
};

/// `PostgreSQL` forge metadata and receive repository.
#[derive(Clone)]
pub struct PgForgeRepository {
    pool: PgPool,
    storage: Arc<GitStorage>,
    authorizer: Option<Arc<PostgresMelangeAuthorizer>>,
}

impl PgForgeRepository {
    /// Creates a repository service.
    #[must_use]
    pub const fn new(pool: PgPool, storage: Arc<GitStorage>) -> Self {
        Self {
            pool,
            storage,
            authorizer: None,
        }
    }

    /// Enables transaction-native run authorization for authenticated receives.
    #[must_use]
    pub fn with_authorizer(mut self, authorizer: Arc<PostgresMelangeAuthorizer>) -> Self {
        self.authorizer = Some(authorizer);
        self
    }

    /// Applies all workspace migrations.
    ///
    /// # Errors
    ///
    /// Returns an error when migration application fails.
    pub async fn initialize(&self) -> Result<(), ForgeRepositoryError> {
        sqlx::migrate!("../../migrations")
            .run(&self.pool)
            .await
            .map_err(storage)?;
        Ok(())
    }

    /// Creates one durable project after checking the owning organization.
    ///
    /// # Errors
    ///
    /// Returns an error for denial, an invalid name, or persistence failure.
    pub async fn create_project(
        &self,
        identity: &AuthenticatedIdentity,
        organization_id: OrganizationId,
        name: &str,
    ) -> Result<Project, ForgeRepositoryError> {
        self.create_project_with_description(identity, organization_id, name, "")
            .await
    }

    /// Creates a durable project with an optional human-readable description
    /// after checking the owning organization.
    ///
    /// The description is stored in the existing project settings JSON object
    /// so this additive metadata does not require a schema migration.
    ///
    /// # Errors
    ///
    /// Returns an error for denial, invalid metadata, or persistence failure.
    pub async fn create_project_with_description(
        &self,
        identity: &AuthenticatedIdentity,
        organization_id: OrganizationId,
        name: &str,
        description: &str,
    ) -> Result<Project, ForgeRepositoryError> {
        validate_name(name, "project name must contain 1 to 200 characters")?;
        validate_description(description)?;
        let authorizer = self
            .authorizer
            .as_ref()
            .ok_or(ForgeRepositoryError::AuthorizationUnavailable)?;
        let mut transaction = begin_actor_transaction(&self.pool, identity)
            .await
            .map_err(storage)?;
        let object = ObjectRef::new(ObjectType::Organization, organization_id.as_uuid());
        let decision = authorizer
            .check(
                &mut transaction,
                Subject::User(identity.user_id),
                Permission::CanCreateProject,
                object,
            )
            .await
            .map_err(storage)?;
        audit_decision(
            &mut transaction,
            identity.user_id,
            Permission::CanCreateProject,
            object,
            decision,
            identity.request_id,
        )
        .await
        .map_err(storage)?;
        if decision == AuthorizationDecision::Deny {
            transaction.commit().await.map_err(storage)?;
            return Err(ForgeRepositoryError::AuthorizationDenied);
        }
        let id = ProjectId::new();
        let row = sqlx::query_as::<_, ProjectRow>(
            "INSERT INTO projects (id, organization_id, name, settings)
             VALUES ($1, $2, $3, $4)
             RETURNING id, organization_id, name, created_at",
        )
        .bind(id.as_uuid())
        .bind(organization_id.as_uuid())
        .bind(name)
        .bind(json!({"description": description}))
        .fetch_one(&mut *transaction)
        .await
        .map_err(storage)?;
        sqlx::query("SELECT ensure_project_maintainer($1, $2)")
            .bind(row.id)
            .bind(identity.user_id.as_uuid())
            .execute(&mut *transaction)
            .await
            .map_err(storage)?;
        transaction.commit().await.map_err(storage)?;
        Ok(row.into())
    }

    /// Creates a project from trusted bootstrap or test code.
    ///
    /// Request-facing code must call [`Self::create_project`].
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid name or persistence failure.
    pub async fn create_project_trusted(
        &self,
        organization_id: OrganizationId,
        name: &str,
    ) -> Result<Project, ForgeRepositoryError> {
        self.create_project_trusted_with_description(organization_id, name, "")
            .await
    }

    /// Creates a project with a description from trusted bootstrap or test
    /// code.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid metadata or persistence failure.
    pub async fn create_project_trusted_with_description(
        &self,
        organization_id: OrganizationId,
        name: &str,
        description: &str,
    ) -> Result<Project, ForgeRepositoryError> {
        validate_name(name, "project name must contain 1 to 200 characters")?;
        validate_description(description)?;
        let id = ProjectId::new();
        let row = sqlx::query_as::<_, ProjectRow>(
            "INSERT INTO projects (id, organization_id, name, settings)
             VALUES ($1, $2, $3, $4)
             RETURNING id, organization_id, name, created_at",
        )
        .bind(id.as_uuid())
        .bind(organization_id.as_uuid())
        .bind(name)
        .bind(json!({"description": description}))
        .fetch_one(&self.pool)
        .await
        .map_err(storage)?;
        Ok(row.into())
    }

    /// Creates metadata after checking the parent project, then initializes its
    /// canonical bare repository.
    ///
    /// A failed Git initialization compensates by removing the uncommitted
    /// metadata row. No caller-supplied path enters the storage operation.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid metadata, storage, or database failures.
    pub async fn create_repository(
        &self,
        identity: &AuthenticatedIdentity,
        input: &CreateRepository,
    ) -> Result<Repository, ForgeRepositoryError> {
        let branch = validate_repository(input)?;
        let authorizer = self
            .authorizer
            .as_ref()
            .ok_or(ForgeRepositoryError::AuthorizationUnavailable)?;
        let mut transaction = begin_actor_transaction(&self.pool, identity)
            .await
            .map_err(storage)?;
        let object = ObjectRef::new(ObjectType::Project, input.project_id.as_uuid());
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
            transaction.commit().await.map_err(storage)?;
            return Err(ForgeRepositoryError::AuthorizationDenied);
        }
        let id = RepositoryId::new();
        let row = insert_repository(&mut transaction, id, input).await?;
        transaction.commit().await.map_err(storage)?;
        if let Err(error) = self.storage.create_bare(id, branch).await {
            sqlx::query("DELETE FROM repositories WHERE id = $1")
                .bind(id.as_uuid())
                .execute(&self.pool)
                .await
                .map_err(storage)?;
            return Err(error.into());
        }
        row.try_into()
    }

    /// Creates a repository from trusted bootstrap or test code.
    ///
    /// Request-facing code must call [`Self::create_repository`].
    ///
    /// # Errors
    ///
    /// Returns an error for invalid metadata, storage, or persistence failure.
    pub async fn create_repository_trusted(
        &self,
        input: &CreateRepository,
    ) -> Result<Repository, ForgeRepositoryError> {
        let branch = validate_repository(input)?;
        let id = RepositoryId::new();
        let mut transaction = self.pool.begin().await.map_err(storage)?;
        let row = insert_repository(&mut transaction, id, input).await?;
        transaction.commit().await.map_err(storage)?;
        if let Err(error) = self.storage.create_bare(id, branch).await {
            sqlx::query("DELETE FROM repositories WHERE id = $1")
                .bind(id.as_uuid())
                .execute(&self.pool)
                .await
                .map_err(storage)?;
            return Err(error.into());
        }
        row.try_into()
    }

    /// Loads repository metadata and verifies its bare storage.
    ///
    /// # Errors
    ///
    /// Returns an error when metadata or canonical storage is absent.
    pub async fn get_repository(
        &self,
        id: RepositoryId,
    ) -> Result<Repository, ForgeRepositoryError> {
        let row = sqlx::query_as::<_, RepositoryRow>(
            "SELECT id, project_id, name, default_branch, is_public, settings, created_at
             FROM repositories WHERE id = $1",
        )
        .bind(id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(storage)?
        .ok_or(ForgeRepositoryError::RepositoryNotFound(id))?;
        self.storage.validate_existing(id).await?;
        row.try_into()
    }

    /// Loads repository metadata in a transaction carrying authenticated actor
    /// context.
    ///
    /// # Errors
    ///
    /// Returns an error when metadata or canonical storage is absent.
    pub async fn get_repository_as(
        &self,
        id: RepositoryId,
        identity: &AuthenticatedIdentity,
    ) -> Result<Repository, ForgeRepositoryError> {
        let mut transaction = begin_actor_transaction(&self.pool, identity)
            .await
            .map_err(storage)?;
        let row = sqlx::query_as::<_, RepositoryRow>(
            "SELECT id, project_id, name, default_branch, is_public, settings, created_at
             FROM repositories WHERE id = $1",
        )
        .bind(id.as_uuid())
        .fetch_optional(&mut *transaction)
        .await
        .map_err(storage)?
        .ok_or(ForgeRepositoryError::RepositoryNotFound(id))?;
        transaction.commit().await.map_err(storage)?;
        self.storage.validate_existing(id).await?;
        row.try_into()
    }

    /// Deletes repository metadata and bare storage after an explicit
    /// `repository.can_delete` check.
    ///
    /// # Errors
    ///
    /// Returns an error for denial or a database/filesystem failure.
    pub async fn delete_repository(
        &self,
        identity: &AuthenticatedIdentity,
        id: RepositoryId,
    ) -> Result<(), ForgeRepositoryError> {
        let authorizer = self
            .authorizer
            .as_ref()
            .ok_or(ForgeRepositoryError::AuthorizationUnavailable)?;
        let mut transaction = begin_actor_transaction(&self.pool, identity)
            .await
            .map_err(storage)?;
        let object = ObjectRef::new(ObjectType::Repository, id.as_uuid());
        let decision = authorizer
            .check(
                &mut transaction,
                Subject::User(identity.user_id),
                Permission::CanDelete,
                object,
            )
            .await
            .map_err(storage)?;
        audit_decision(
            &mut transaction,
            identity.user_id,
            Permission::CanDelete,
            object,
            decision,
            identity.request_id,
        )
        .await
        .map_err(storage)?;
        if decision == AuthorizationDecision::Deny {
            transaction.commit().await.map_err(storage)?;
            return Err(ForgeRepositoryError::AuthorizationDenied);
        }
        let deleted = sqlx::query("DELETE FROM repositories WHERE id = $1")
            .bind(id.as_uuid())
            .execute(&mut *transaction)
            .await
            .map_err(storage)?;
        if deleted.rows_affected() == 0 {
            return Err(ForgeRepositoryError::RepositoryNotFound(id));
        }
        transaction.commit().await.map_err(storage)?;
        self.storage.delete_bare(id).await?;
        Ok(())
    }

    /// Records an accepted receive and derives configuration revisions and run
    /// requests in one database transaction.
    ///
    /// The exact new commit of each non-delete update is inspected before the
    /// transaction begins. All resulting audit records, revisions, requests,
    /// and outbox events are then committed atomically.
    ///
    /// # Errors
    ///
    /// Returns an error when Git inspection, serialization, or persistence
    /// fails. No partial database effects are committed.
    pub async fn accept_receive(
        &self,
        repository: &Repository,
        receive_id: ReceiveId,
        principal: &str,
        updates: &[RefUpdate],
    ) -> Result<ReceiveResult, ForgeRepositoryError> {
        self.accept_receive_as(repository, receive_id, principal, None, updates)
            .await
    }

    /// Records an accepted receive with authenticated actor provenance.
    ///
    /// # Errors
    ///
    /// Returns the same failures as [`Self::accept_receive`].
    // Keeping this transactional workflow together makes the all-or-nothing
    // receive invariant directly auditable.
    #[allow(clippy::too_many_lines)]
    // The receive transaction deliberately keeps authorization, ref
    // validation, persistence, and outbox publication in one auditable path.
    #[allow(clippy::cognitive_complexity)]
    pub async fn accept_receive_as(
        &self,
        repository: &Repository,
        receive_id: ReceiveId,
        principal: &str,
        identity: Option<&AuthenticatedIdentity>,
        updates: &[RefUpdate],
    ) -> Result<ReceiveResult, ForgeRepositoryError> {
        if identity.is_some() && self.authorizer.is_none() {
            return Err(ForgeRepositoryError::AuthorizationUnavailable);
        }
        let repository_path = self.storage.validate_existing(repository.id).await?;
        let mut transaction = match identity {
            Some(identity) => begin_actor_transaction(&self.pool, identity)
                .await
                .map_err(storage)?,
            None => self.pool.begin().await.map_err(storage)?,
        };
        let now = OffsetDateTime::now_utc();
        let inserted = sqlx::query(
            "INSERT INTO git_receives
             (id, repository_id, actor_id, principal, request_id,
              status, accepted_at, created_at)
             VALUES ($1, $2, $3, $4, $5, 'accepted', $6, $6)
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(receive_id.as_uuid())
        .bind(repository.id.as_uuid())
        .bind(identity.map(|value| value.user_id.as_uuid()))
        .bind(principal)
        .bind(identity.map(|value| value.request_id.as_uuid()))
        .bind(now)
        .execute(&mut *transaction)
        .await
        .map_err(storage)?;
        if inserted.rows_affected() == 0 {
            let existing_repository: Uuid =
                sqlx::query_scalar("SELECT repository_id FROM git_receives WHERE id = $1")
                    .bind(receive_id.as_uuid())
                    .fetch_one(&mut *transaction)
                    .await
                    .map_err(storage)?;
            if existing_repository != repository.id.as_uuid() {
                return Err(ForgeRepositoryError::ReceiveConflict(receive_id));
            }
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
            let invalid_configurations = sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM agent_config_revisions
                 WHERE receive_id = $1 AND status = 'invalid'",
            )
            .bind(receive_id.as_uuid())
            .fetch_one(&mut *transaction)
            .await
            .map_err(storage)?;
            let build_requests = sqlx::query_scalar::<_, Uuid>(
                "SELECT source.build_request_id
                 FROM build_request_sources AS source
                 WHERE source.receive_id = $1
                 ORDER BY source.created_at, source.build_request_id",
            )
            .bind(receive_id.as_uuid())
            .fetch_all(&mut *transaction)
            .await
            .map_err(storage)?
            .into_iter()
            .map(BuildRequestId::from_uuid)
            .collect();
            transaction.commit().await.map_err(storage)?;
            return Ok(ReceiveResult {
                receive_id,
                run_requests,
                build_requests,
                invalid_configurations: usize::try_from(invalid_configurations).map_err(|_| {
                    ForgeRepositoryError::InvalidStoredData("invalid revision count")
                })?,
            });
        }
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
                build_requests.push(
                    persist_build_request(
                        &mut transaction,
                        repository.id,
                        receive_id,
                        identity,
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
                        now,
                    )
                    .await?,
                );
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

    /// Loads unpublished actionable command entries.
    ///
    /// # Errors
    ///
    /// Returns an error when `PostgreSQL` access fails.
    pub async fn unpublished_outbox(
        &self,
        limit: i64,
    ) -> Result<Vec<OutboxRecord>, ForgeRepositoryError> {
        let rows = sqlx::query_as::<_, OutboxRow>(
            "SELECT id, subject, payload FROM outbox
             WHERE published_at IS NULL
               AND subject IN (
                   'hephaestus.build.requested.v1',
                   'hephaestus.instance.run.requested.v1',
                   'hephaestus.run.start'
               )
             ORDER BY occurred_at, id LIMIT $1",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(storage)?;
        Ok(rows
            .into_iter()
            .map(|row| OutboxRecord {
                id: EventId::from_uuid(row.id),
                subject: row.subject,
                payload: row.payload,
            })
            .collect())
    }

    /// Marks an outbox event as acknowledged.
    ///
    /// # Errors
    ///
    /// Returns an error when `PostgreSQL` access fails.
    pub async fn mark_outbox_published(
        &self,
        event_id: EventId,
    ) -> Result<(), ForgeRepositoryError> {
        sqlx::query("UPDATE outbox SET published_at = now() WHERE id = $1")
            .bind(event_id.as_uuid())
            .execute(&self.pool)
            .await
            .map_err(storage)?;
        Ok(())
    }

    /// Records a failed publication attempt.
    ///
    /// # Errors
    ///
    /// Returns an error when `PostgreSQL` access fails.
    pub async fn mark_outbox_failed(
        &self,
        event_id: EventId,
        error: &str,
    ) -> Result<(), ForgeRepositoryError> {
        sqlx::query("UPDATE outbox SET attempts = attempts + 1, last_error = $2 WHERE id = $1")
            .bind(event_id.as_uuid())
            .bind(error)
            .execute(&self.pool)
            .await
            .map_err(storage)?;
        Ok(())
    }
}

fn validate_name(name: &str, message: &'static str) -> Result<(), ForgeRepositoryError> {
    if name.trim().is_empty() || name.len() > 200 {
        Err(ForgeRepositoryError::InvalidMetadata(message))
    } else {
        Ok(())
    }
}

fn validate_description(description: &str) -> Result<(), ForgeRepositoryError> {
    if description.chars().count() > 2_000 {
        Err(ForgeRepositoryError::InvalidMetadata(
            "project description must contain at most 2000 characters",
        ))
    } else {
        Ok(())
    }
}

fn validate_repository(input: &CreateRepository) -> Result<&str, ForgeRepositoryError> {
    validate_name(
        &input.name,
        "repository name must contain 1 to 200 characters",
    )?;
    input
        .default_branch
        .as_str()
        .strip_prefix("refs/heads/")
        .ok_or(ForgeRepositoryError::InvalidMetadata(
            "default branch must be beneath refs/heads/",
        ))
}

async fn insert_repository(
    transaction: &mut Transaction<'_, Postgres>,
    id: RepositoryId,
    input: &CreateRepository,
) -> Result<RepositoryRow, ForgeRepositoryError> {
    sqlx::query_as::<_, RepositoryRow>(
        "INSERT INTO repositories
         (id, project_id, name, default_branch, is_public, settings)
         VALUES ($1, $2, $3, $4, $5, jsonb_build_object('agent_runs_enabled', $6))
         RETURNING id, project_id, name, default_branch, is_public, settings, created_at",
    )
    .bind(id.as_uuid())
    .bind(input.project_id.as_uuid())
    .bind(&input.name)
    .bind(input.default_branch.as_str())
    .bind(input.is_public)
    .bind(input.agent_runs_enabled)
    .fetch_one(&mut **transaction)
    .await
    .map_err(storage)
}

#[derive(Debug)]
struct InspectedUpdate {
    git_ref: GitRef,
    commit: CommitSha,
    parsed: Option<ParsedConfig>,
    repository_oci_images: Option<Vec<InspectedRepositoryOciImage>>,
}

#[derive(Debug)]
struct InspectedRepositoryOciImage {
    key: String,
    display_name: String,
    dockerfile_path: String,
    context_path: String,
    context_digest: String,
    base_key: String,
}

fn inspect_updates(
    repository_path: &Path,
    updates: &[RefUpdate],
) -> Result<Vec<InspectedUpdate>, ForgeRepositoryError> {
    let repository = gix::open(repository_path).map_err(git)?;
    updates
        .iter()
        .filter_map(|update| {
            update.new_commit.as_ref().map(|commit| {
                let object_id = gix::ObjectId::from_hex(commit.as_str().as_bytes()).map_err(git)?;
                let object = repository.find_object(object_id).map_err(git)?;
                let tree = object.try_into_commit().map_err(git)?.tree().map_err(git)?;
                let parsed = tree
                    .lookup_entry_by_path("agent.toml")
                    .map_err(git)?
                    .map(|entry| entry.object().map_err(git))
                    .transpose()?
                    .map(|object| {
                        object
                            .try_into_blob()
                            .map(|blob| agent_config::parse(&blob.data))
                            .map_err(git)
                    })
                    .transpose()?;
                let repository_oci_images = inspect_repository_oci_images(&tree)?;
                Ok(InspectedUpdate {
                    git_ref: update.git_ref.clone(),
                    commit: commit.clone(),
                    parsed,
                    repository_oci_images,
                })
            })
        })
        .collect()
}

fn inspect_repository_oci_images(
    tree: &gix::Tree<'_>,
) -> Result<Option<Vec<InspectedRepositoryOciImage>>, ForgeRepositoryError> {
    let Some(entry) = tree.lookup_entry_by_path("heph.images.toml").map_err(git)? else {
        return Ok(None);
    };
    if !entry.mode().is_blob() {
        return Err(ForgeRepositoryError::InvalidMetadata(
            "heph.images.toml must be a regular file",
        ));
    }
    let manifest = entry.object().map_err(git)?.try_into_blob().map_err(git)?;
    let parsed = agent_config::parse_repository_oci_images(&manifest.data);
    let Some(config) = parsed.config else {
        return Ok(None);
    };
    config
        .images
        .into_iter()
        .map(|image| inspect_repository_oci_image(tree, image))
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}

fn inspect_repository_oci_image(
    tree: &gix::Tree<'_>,
    image: agent_config::RepositoryOciImageConfig,
) -> Result<InspectedRepositoryOciImage, ForgeRepositoryError> {
    let dockerfile = tree
        .lookup_entry_by_path(&image.build.dockerfile)
        .map_err(git)?
        .ok_or(ForgeRepositoryError::InvalidMetadata(
            "repository OCI image Dockerfile does not exist in its source commit",
        ))?;
    if !dockerfile.mode().is_blob() {
        return Err(ForgeRepositoryError::InvalidMetadata(
            "repository OCI image Dockerfile must be a regular file, not a symlink",
        ));
    }
    let context = if image.build.context == "." {
        tree.clone()
    } else {
        let entry = tree
            .lookup_entry_by_path(&image.build.context)
            .map_err(git)?
            .ok_or(ForgeRepositoryError::InvalidMetadata(
                "repository OCI image context does not exist in its source commit",
            ))?;
        if !entry.mode().is_tree() {
            return Err(ForgeRepositoryError::InvalidMetadata(
                "repository OCI image context must be a directory, not a symlink",
            ));
        }
        entry.object().map_err(git)?.try_into_tree().map_err(git)?
    };
    validate_repository_oci_image_context(&context)?;
    let context_digest = format!("sha256:{:x}", Sha256::digest(&context.data));
    Ok(InspectedRepositoryOciImage {
        key: image.key,
        display_name: image.display_name,
        dockerfile_path: image.build.dockerfile,
        context_path: image.build.context,
        context_digest,
        base_key: image.build.base.key,
    })
}

fn validate_repository_oci_image_context(tree: &gix::Tree<'_>) -> Result<(), ForgeRepositoryError> {
    for entry in tree.iter() {
        let entry = entry.map_err(git)?;
        if entry.mode().is_link() || entry.mode().is_commit() {
            return Err(ForgeRepositoryError::InvalidMetadata(
                "repository OCI image contexts may not contain symlinks or submodules",
            ));
        }
        if entry.mode().is_tree() {
            let child = entry.object().map_err(git)?.try_into_tree().map_err(git)?;
            validate_repository_oci_image_context(&child)?;
        }
    }
    Ok(())
}

async fn persist_revision(
    transaction: &mut Transaction<'_, Postgres>,
    repository_id: RepositoryId,
    receive_id: ReceiveId,
    commit: &CommitSha,
    parsed: &ParsedConfig,
    now: OffsetDateTime,
) -> Result<AgentConfigRevisionId, ForgeRepositoryError> {
    let revision_id = AgentConfigRevisionId::new();
    let status = if parsed.config.is_some() {
        "valid"
    } else {
        "invalid"
    };
    let config = parsed
        .config
        .as_ref()
        .map(serde_json::to_value)
        .transpose()
        .map_err(serialization)?;
    let diagnostics = serde_json::to_value(&parsed.diagnostics).map_err(serialization)?;
    let row = sqlx::query_as::<_, RevisionIdentityRow>(
        "INSERT INTO agent_config_revisions
         (id, repository_id, receive_id, commit_sha, config_hash, schema_version,
          status, config, diagnostics, created_at,
          normalized_config_hash)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
         ON CONFLICT (repository_id, commit_sha, config_hash)
         DO UPDATE SET repository_id = EXCLUDED.repository_id
         RETURNING id",
    )
    .bind(revision_id.as_uuid())
    .bind(repository_id.as_uuid())
    .bind(receive_id.as_uuid())
    .bind(commit.as_str())
    .bind(parsed.hash.as_str())
    .bind(
        parsed
            .config
            .as_ref()
            .map(|config| i32::try_from(config.version).unwrap_or(i32::MAX)),
    )
    .bind(status)
    .bind(config)
    .bind(diagnostics)
    .bind(now)
    .bind(parsed.normalized_hash.as_ref().map(ConfigHash::as_str))
    .fetch_one(&mut **transaction)
    .await
    .map_err(storage)?;
    Ok(AgentConfigRevisionId::from_uuid(row.id))
}

async fn persist_repository_oci_image_revisions(
    transaction: &mut Transaction<'_, Postgres>,
    repository_id: RepositoryId,
    project_id: ProjectId,
    source_commit: &CommitSha,
    images: &[InspectedRepositoryOciImage],
) -> Result<(), ForgeRepositoryError> {
    for image in images {
        // Keeping the catalog read and revision insert in the receive
        // transaction makes the selected base digest part of the immutable
        // source revision, rather than resolving a mutable key later.
        let base_reference: Option<String> = sqlx::query_scalar(
            "SELECT image_reference
             FROM oci_images
             WHERE key = $1 AND availability_state = 'available'
             FOR SHARE",
        )
        .bind(&image.base_key)
        .fetch_optional(&mut **transaction)
        .await
        .map_err(storage)?;
        let base_reference = base_reference.ok_or(ForgeRepositoryError::InvalidMetadata(
            "repository OCI image base is not an available catalog image",
        ))?;
        let revision_id = Uuid::new_v4();
        sqlx::query_scalar::<_, Uuid>(
            "INSERT INTO repository_oci_image_definitions
                (id, project_id, source_repository_id, key, display_name,
                 source_revision, dockerfile_path, context_path, context_digest,
                 base_image_reference, status)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, 'producing')
             ON CONFLICT (source_repository_id, key, source_revision,
                          context_digest, base_image_reference)
             DO NOTHING
             RETURNING id",
        )
        .bind(revision_id)
        .bind(project_id.as_uuid())
        .bind(repository_id.as_uuid())
        .bind(&image.key)
        .bind(&image.display_name)
        .bind(source_commit.as_str())
        .bind(&image.dockerfile_path)
        .bind(&image.context_path)
        .bind(&image.context_digest)
        .bind(base_reference)
        .fetch_optional(&mut **transaction)
        .await
        .map_err(storage)?;
    }
    Ok(())
}

fn build_trigger_matches(patterns: &[String], git_ref: &GitRef) -> bool {
    patterns.iter().any(|pattern| {
        pattern.strip_suffix("/*").map_or_else(
            || pattern == git_ref.as_str(),
            |prefix| {
                git_ref
                    .as_str()
                    .strip_prefix(prefix)
                    .is_some_and(|suffix| suffix.starts_with('/'))
            },
        )
    })
}

#[allow(clippy::too_many_arguments)]
async fn persist_build_request(
    transaction: &mut Transaction<'_, Postgres>,
    repository_id: RepositoryId,
    receive_id: ReceiveId,
    identity: Option<&AuthenticatedIdentity>,
    git_ref: &GitRef,
    commit: &CommitSha,
    build: &agent_config::BuildConfig,
    guest_image: &agent_config::ImageSelection,
    agent_key: Option<&str>,
    normalized_hash: &ConfigHash,
    now: OffsetDateTime,
) -> Result<BuildRequestId, ForgeRepositoryError> {
    let build_definition =
        serde_json::to_vec(build).map_err(ForgeRepositoryError::Serialization)?;
    let build_declaration =
        serde_json::to_value(build).map_err(ForgeRepositoryError::Serialization)?;
    let build_policy = json!({
        "resources": build.resources,
        "network": build.network,
    });
    let declared_artifacts =
        serde_json::to_value(&build.artifacts).map_err(ForgeRepositoryError::Serialization)?;
    let build_definition_hash: [u8; 32] = Sha256::digest(&build_definition).into();
    let build_image = resolve_image(transaction, &build.image.key).await?;
    let guest_image = resolve_image(transaction, &guest_image.key).await?;
    let requested_id = BuildRequestId::new();
    let stored_id: Uuid = sqlx::query_scalar(
        "INSERT INTO build_requests
         (id, repository_id, source_commit, source_ref, origin_receive_id,
          build_definition_hash, state, created_by, created_at, build_trigger,
          agent_key, configuration_hash, build_declaration, build_policy,
          declared_artifacts)
         VALUES ($1, $2, $3, $4, $5, $6, 'queued', $7, $8, 'push', $9,
                 decode($10, 'hex'), $11, $12, $13)
         ON CONFLICT (
             repository_id, source_commit, source_ref, build_definition_hash
         ) DO UPDATE SET repository_id = EXCLUDED.repository_id
         RETURNING id",
    )
    .bind(requested_id.as_uuid())
    .bind(repository_id.as_uuid())
    .bind(commit.as_str())
    .bind(git_ref.as_str())
    .bind(receive_id.as_uuid())
    .bind(build_definition_hash.as_slice())
    .bind(identity.map(|value| value.user_id.as_uuid()))
    .bind(now)
    .bind(agent_key)
    .bind(normalized_hash.as_str())
    .bind(build_declaration)
    .bind(build_policy)
    .bind(declared_artifacts)
    .fetch_one(&mut **transaction)
    .await
    .map_err(storage)?;
    for (execution_context, image) in [("build", &build_image), ("guest", &guest_image)] {
        sqlx::query(
            "INSERT INTO build_request_images
                (build_request_id, execution_context, image_id, image_key, image_reference)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT DO NOTHING",
        )
        .bind(stored_id)
        .bind(execution_context)
        .bind(image.id)
        .bind(&image.key)
        .bind(&image.image_reference)
        .execute(&mut **transaction)
        .await
        .map_err(storage)?;
    }
    sqlx::query(
        "INSERT INTO build_request_sources
         (build_request_id, receive_id, source_ref, source_commit, created_at)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT DO NOTHING",
    )
    .bind(stored_id)
    .bind(receive_id.as_uuid())
    .bind(git_ref.as_str())
    .bind(commit.as_str())
    .bind(now)
    .execute(&mut **transaction)
    .await
    .map_err(storage)?;
    append_outbox(
        transaction,
        stored_id,
        BUILD_REQUESTED_SUBJECT,
        "build.requested.v1",
        json!({
            "schema_version": 1,
            "build_request_id": stored_id,
            "repository_id": repository_id,
            "source_commit": commit,
            "source_ref": git_ref,
            "receive_id": receive_id,
            "normalized_configuration_hash": normalized_hash,
            "build_definition_hash": hex_digest(&build_definition_hash),
        }),
        now,
    )
    .await?;
    Ok(BuildRequestId::from_uuid(stored_id))
}

#[derive(Debug, sqlx::FromRow)]
struct ResolvedImageRow {
    id: Uuid,
    key: String,
    image_reference: String,
}

async fn resolve_image(
    transaction: &mut Transaction<'_, Postgres>,
    key: &str,
) -> Result<ResolvedImageRow, ForgeRepositoryError> {
    sqlx::query_as::<_, ResolvedImageRow>(
        "SELECT id, key, image_reference
           FROM oci_images
          WHERE key = $1 AND availability_state = 'available'
          FOR SHARE",
    )
    .bind(key)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(storage)?
    .ok_or(ForgeRepositoryError::InvalidMetadata(
        "selected OCI image is unavailable",
    ))
}

fn hex_digest(digest: &[u8; 32]) -> String {
    use std::fmt::Write as _;

    let mut output = String::with_capacity(64);
    for byte in digest {
        write!(&mut output, "{byte:02x}").expect("writing to a String cannot fail");
    }
    output
}

#[derive(sqlx::FromRow)]
struct InstanceTriggerRow {
    attachment_id: Uuid,
    instance_id: Uuid,
    instance_revision_id: Uuid,
    release_id: Uuid,
    release_agent_id: Uuid,
    platform_policy_version: String,
    run_gate_open: bool,
    instance_state: String,
    runnable: bool,
    requires_state: bool,
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
async fn persist_instance_triggers(
    transaction: &mut Transaction<'_, Postgres>,
    repository: &Repository,
    receive_id: ReceiveId,
    identity: Option<&AuthenticatedIdentity>,
    updates: &[RefUpdate],
    authorizer: Option<&PostgresMelangeAuthorizer>,
    now: OffsetDateTime,
) -> Result<(), ForgeRepositoryError> {
    if !repository.agent_runs_enabled {
        return Ok(());
    }
    for update in updates {
        let Some(commit) = &update.new_commit else {
            continue;
        };
        let candidates: Vec<InstanceTriggerRow> = sqlx::query_as(
            "SELECT attachment.id AS attachment_id,
                    instance.id AS instance_id,
                    revision.id AS instance_revision_id,
                    release.id AS release_id,
                    release_agent.id AS release_agent_id,
                    revision.platform_policy_version,
                    instance.run_gate_open, instance.state AS instance_state,
                    revision.runnable
                    , release_agent.requires_state
             FROM agent_attachments AS attachment
             JOIN agent_instances AS instance
               ON instance.id = attachment.instance_id
             JOIN agent_instance_revisions AS revision
               ON revision.id = instance.active_revision_id
             JOIN release_agents AS release_agent
               ON release_agent.id = revision.release_agent_id
             JOIN releases AS release ON release.id = release_agent.release_id
             WHERE attachment.repository_id = $1
               AND attachment.enabled AND attachment.removed_at IS NULL
               AND attachment.trigger_policy IN ('push', 'push_and_manual')
               AND release.state = 'published'
               AND (
                   attachment.ref_selector = $2
                   OR (
                       right(attachment.ref_selector, 2) = '/*'
                       AND $2 LIKE
                           left(
                               attachment.ref_selector,
                               length(attachment.ref_selector) - 1
                           ) || '%'
                   )
               )
             ORDER BY attachment.id",
        )
        .bind(repository.id.as_uuid())
        .bind(update.git_ref.as_str())
        .fetch_all(&mut **transaction)
        .await
        .map_err(storage)?;
        for candidate in candidates {
            let mut permitted = true;
            if let (Some(identity), Some(authorizer)) = (identity, authorizer) {
                for (permission, object) in [
                    (
                        Permission::CanExecute,
                        ObjectRef::new(ObjectType::AgentAttachment, candidate.attachment_id),
                    ),
                    (
                        Permission::CanUse,
                        ObjectRef::new(ObjectType::ReleaseAgent, candidate.release_agent_id),
                    ),
                ] {
                    let decision = authorizer
                        .check(
                            transaction,
                            Subject::User(identity.user_id),
                            permission,
                            object,
                        )
                        .await
                        .map_err(storage)?;
                    audit_decision(
                        transaction,
                        identity.user_id,
                        permission,
                        object,
                        decision,
                        identity.request_id,
                    )
                    .await
                    .map_err(storage)?;
                    if decision == AuthorizationDecision::Deny {
                        permitted = false;
                        break;
                    }
                }
            }
            if !permitted {
                continue;
            }
            if !candidate.run_gate_open {
                sqlx::query(
                    "INSERT INTO deferred_agent_triggers
                     (id, instance_id, attachment_id, repository_id,
                      target_ref, target_commit, source_id)
                     VALUES ($1, $2, $3, $4, $5, $6, $7)
                     ON CONFLICT (
                         attachment_id, repository_id, target_ref,
                         target_commit, source_id
                     ) DO NOTHING",
                )
                .bind(Uuid::new_v4())
                .bind(candidate.instance_id)
                .bind(candidate.attachment_id)
                .bind(repository.id.as_uuid())
                .bind(update.git_ref.as_str())
                .bind(commit.as_str())
                .bind(receive_id.as_uuid())
                .execute(&mut **transaction)
                .await
                .map_err(storage)?;
                continue;
            }
            if !candidate.runnable
                || !["active", "update_rejected"].contains(&candidate.instance_state.as_str())
            {
                continue;
            }
            let request_id = RunRequestId::new();
            let run_id = RunId::new();
            let command_id = CommandId::new();
            let stored: (Uuid, Uuid, Uuid) = sqlx::query_as(
                "INSERT INTO run_requests
                 (id, repository_id, commit_sha, git_ref, receive_id,
                  run_id, command_id, actor_id, request_id, created_at,
                  instance_id, instance_revision_id, release_id,
                  release_agent_id, attachment_id, request_kind,
                  platform_policy_version, requires_state)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10,
                         $11, $12, $13, $14, $15, 'instance_normal', $16, $17)
                 ON CONFLICT (
                     attachment_id, instance_revision_id, commit_sha, git_ref,
                     receive_id, attempt
                 ) WHERE request_kind = 'instance_normal'
                 DO UPDATE SET repository_id = EXCLUDED.repository_id
                 RETURNING id, run_id, command_id",
            )
            .bind(request_id.as_uuid())
            .bind(repository.id.as_uuid())
            .bind(commit.as_str())
            .bind(update.git_ref.as_str())
            .bind(receive_id.as_uuid())
            .bind(run_id.as_uuid())
            .bind(command_id.as_uuid())
            .bind(identity.map(|value| value.user_id.as_uuid()))
            .bind(identity.map(|value| value.request_id.as_uuid()))
            .bind(now)
            .bind(candidate.instance_id)
            .bind(candidate.instance_revision_id)
            .bind(candidate.release_id)
            .bind(candidate.release_agent_id)
            .bind(candidate.attachment_id)
            .bind(&candidate.platform_policy_version)
            .bind(candidate.requires_state)
            .fetch_one(&mut **transaction)
            .await
            .map_err(storage)?;
            append_outbox(
                transaction,
                stored.0,
                INSTANCE_RUN_REQUESTED_SUBJECT,
                "instance.run.requested.v1",
                json!({
                    "schema_version": 1,
                    "run_request_id": stored.0,
                    "run_id": stored.1,
                    "command_id": stored.2,
                    "receive_id": receive_id,
                    "instance_id": candidate.instance_id,
                    "instance_revision_id": candidate.instance_revision_id,
                    "release_id": candidate.release_id,
                    "release_agent_id": candidate.release_agent_id,
                    "attachment_id": candidate.attachment_id,
                    "target_repository_id": repository.id,
                    "target_ref": update.git_ref,
                    "target_commit": commit,
                    "platform_policy_version": candidate.platform_policy_version,
                    "requires_state": candidate.requires_state,
                }),
                now,
            )
            .await?;
            let command = StartRun {
                command_id: CommandId::from_uuid(stored.2),
                run_id: RunId::from_uuid(stored.1),
                instance_id: AgentInstanceId::from_uuid(candidate.instance_id),
                instance_revision_id: AgentInstanceRevisionId::from_uuid(
                    candidate.instance_revision_id,
                ),
                release_id: ReleaseId::from_uuid(candidate.release_id),
                release_agent_id: ReleaseAgentId::from_uuid(candidate.release_agent_id),
                attachment_id: Some(AgentAttachmentId::from_uuid(candidate.attachment_id)),
                kind: RunKind::Normal,
                requires_state: candidate.requires_state,
            };
            append_outbox(
                transaction,
                stored.1,
                RUN_START_SUBJECT,
                "run.start.v1",
                serde_json::to_value(command).map_err(serialization)?,
                now,
            )
            .await?;
        }
    }
    Ok(())
}

async fn append_outbox(
    transaction: &mut Transaction<'_, Postgres>,
    aggregate_id: Uuid,
    subject: &str,
    event_type: &str,
    mut payload: Value,
    occurred_at: OffsetDateTime,
) -> Result<(), ForgeRepositoryError> {
    let event_id = EventId::new();
    if let Some(object) = payload.as_object_mut() {
        object
            .entry(String::from("schema_version"))
            .or_insert_with(|| json!(1));
        object
            .entry(String::from("message_id"))
            .or_insert_with(|| json!(event_id));
        object
            .entry(String::from("idempotency_key"))
            .or_insert_with(|| json!(event_id));
        object
            .entry(String::from("request_id"))
            .or_insert(Value::Null);
        object
            .entry(String::from("trace_id"))
            .or_insert(Value::Null);
    }
    sqlx::query(
        "INSERT INTO outbox
         (id, aggregate_type, aggregate_id, subject, event_type, payload, occurred_at)
         VALUES ($1, 'forge', $2, $3, $4, $5, $6)",
    )
    .bind(event_id.as_uuid())
    .bind(aggregate_id)
    .bind(subject)
    .bind(event_type)
    .bind(payload)
    .bind(occurred_at)
    .execute(&mut **transaction)
    .await
    .map_err(storage)?;
    Ok(())
}

#[derive(FromRow)]
struct ProjectRow {
    id: Uuid,
    organization_id: Uuid,
    name: String,
    created_at: OffsetDateTime,
}

impl From<ProjectRow> for Project {
    fn from(row: ProjectRow) -> Self {
        Self {
            id: ProjectId::from_uuid(row.id),
            organization_id: OrganizationId::from_uuid(row.organization_id),
            name: row.name,
            created_at: row.created_at,
        }
    }
}

#[derive(FromRow)]
struct RepositoryRow {
    id: Uuid,
    project_id: Uuid,
    name: String,
    default_branch: String,
    is_public: bool,
    settings: Value,
    created_at: OffsetDateTime,
}

impl TryFrom<RepositoryRow> for Repository {
    type Error = ForgeRepositoryError;

    fn try_from(row: RepositoryRow) -> Result<Self, Self::Error> {
        Ok(Self {
            id: RepositoryId::from_uuid(row.id),
            project_id: ProjectId::from_uuid(row.project_id),
            name: row.name,
            default_branch: GitRef::parse(row.default_branch)
                .map_err(|_| ForgeRepositoryError::InvalidStoredData("default_branch"))?,
            is_public: row.is_public,
            agent_runs_enabled: row
                .settings
                .get("agent_runs_enabled")
                .and_then(Value::as_bool)
                .unwrap_or(true),
            created_at: row.created_at,
        })
    }
}

#[derive(FromRow)]
struct RevisionIdentityRow {
    id: Uuid,
}

#[derive(FromRow)]
struct RunRequestRow {
    id: Uuid,
    repository_id: Uuid,
    commit_sha: String,
    git_ref: String,
    receive_id: Uuid,
    instance_id: Uuid,
    instance_revision_id: Uuid,
    release_id: Uuid,
    release_agent_id: Uuid,
    attachment_id: Uuid,
    run_id: Uuid,
    command_id: Uuid,
    requires_state: bool,
}

impl TryFrom<RunRequestRow> for RunRequest {
    type Error = ForgeRepositoryError;

    fn try_from(row: RunRequestRow) -> Result<Self, Self::Error> {
        Ok(Self {
            id: RunRequestId::from_uuid(row.id),
            repository_id: RepositoryId::from_uuid(row.repository_id),
            commit_sha: CommitSha::parse(row.commit_sha)
                .map_err(|_| ForgeRepositoryError::InvalidStoredData("commit_sha"))?,
            git_ref: GitRef::parse(row.git_ref)
                .map_err(|_| ForgeRepositoryError::InvalidStoredData("git_ref"))?,
            receive_id: ReceiveId::from_uuid(row.receive_id),
            command: StartRun {
                command_id: CommandId::from_uuid(row.command_id),
                run_id: RunId::from_uuid(row.run_id),
                instance_id: AgentInstanceId::from_uuid(row.instance_id),
                instance_revision_id: AgentInstanceRevisionId::from_uuid(row.instance_revision_id),
                release_id: ReleaseId::from_uuid(row.release_id),
                release_agent_id: ReleaseAgentId::from_uuid(row.release_agent_id),
                attachment_id: Some(AgentAttachmentId::from_uuid(row.attachment_id)),
                kind: RunKind::Normal,
                requires_state: row.requires_state,
            },
        })
    }
}

#[derive(FromRow)]
struct OutboxRow {
    id: Uuid,
    subject: String,
    payload: Value,
}

fn storage(error: impl std::error::Error + Send + Sync + 'static) -> ForgeRepositoryError {
    ForgeRepositoryError::Storage(Box::new(error))
}

fn git(error: impl std::fmt::Display) -> ForgeRepositoryError {
    ForgeRepositoryError::GitInspection(error.to_string())
}

const fn serialization(error: serde_json::Error) -> ForgeRepositoryError {
    ForgeRepositoryError::Serialization(error)
}

#[cfg(test)]
mod tests {
    use super::inspect_updates;
    use forge_domain::{CommitSha, GitRef, RefUpdate};
    use std::{path::Path, process::Command};

    #[test]
    fn discovers_repository_oci_image_inputs_from_the_exact_received_commit() {
        let temporary = tempfile::tempdir().expect("temporary repository");
        let bare = temporary.path().join("source.git");
        let worktree = temporary.path().join("source");
        git(
            temporary.path(),
            &["init", "--bare", bare.to_str().expect("UTF-8 path")],
        );
        git(
            temporary.path(),
            &["init", worktree.to_str().expect("UTF-8 path")],
        );
        git(&worktree, &["config", "user.name", "Hephaestus Test"]);
        git(
            &worktree,
            &["config", "user.email", "hephaestus@example.invalid"],
        );
        std::fs::create_dir_all(worktree.join("containers")).expect("containers directory");
        std::fs::write(
            worktree.join("heph.images.toml"),
            r#"
version = 1

[[images]]
key = "typescript-tools"
display_name = "TypeScript tools"

[images.build]
dockerfile = "containers/Dockerfile"
context = "."
base = { key = "typescript-node-ubuntu" }
"#,
        )
        .expect("manifest");
        std::fs::write(
            worktree.join("containers/Dockerfile"),
            "FROM heph-base AS build\nRUN echo ready\n",
        )
        .expect("Dockerfile");
        git(&worktree, &["add", "."]);
        git(&worktree, &["commit", "-m", "repository OCI image"]);
        let commit =
            CommitSha::parse(git_output(&worktree, &["rev-parse", "HEAD"])).expect("commit ID");
        git(
            &worktree,
            &[
                "push",
                bare.to_str().expect("UTF-8 path"),
                "HEAD:refs/heads/main",
            ],
        );

        let inspected = inspect_updates(
            &bare,
            &[RefUpdate {
                git_ref: GitRef::parse("refs/heads/main").expect("Git ref"),
                old_commit: None,
                new_commit: Some(commit),
            }],
        )
        .expect("inspect received commit");

        let images = inspected[0]
            .repository_oci_images
            .as_ref()
            .expect("repository OCI image manifest");
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].key, "typescript-tools");
        assert_eq!(images[0].dockerfile_path, "containers/Dockerfile");
        assert_eq!(images[0].context_digest.len(), 71);
        assert!(images[0].context_digest.starts_with("sha256:"));
    }

    fn git(directory: &Path, arguments: &[&str]) {
        let output = Command::new("git")
            .args(arguments)
            .current_dir(directory)
            .output()
            .expect("run Git");
        assert!(
            output.status.success(),
            "git {arguments:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn git_output(directory: &Path, arguments: &[&str]) -> String {
        let output = Command::new("git")
            .args(arguments)
            .current_dir(directory)
            .output()
            .expect("run Git");
        assert!(output.status.success(), "git {arguments:?} failed");
        String::from_utf8(output.stdout)
            .expect("UTF-8 Git output")
            .trim()
            .to_owned()
    }
}
