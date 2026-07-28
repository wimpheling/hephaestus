use agent_config::{ConfigHash, ParsedConfig};
use authz_domain::{AuthorizationDecision, Authorizer, ObjectRef, ObjectType, Permission, Subject};
use authz_postgres::{audit_decision, begin_actor_transaction};
use forge_domain::{
    AgentConfigRevisionId, CommitSha, GitRef, OrganizationId, Project, ProjectId, ReceiveId,
    RefUpdate, Repository, RepositoryId, RunRequestId,
};
use identity_domain::AuthenticatedIdentity;
use run_domain::StartRun;
use runtime_types::{AgentId, CommandId, EventId, RunId};
use serde_json::{Value, json};
use sqlx::{FromRow, PgPool, Postgres, Transaction};
use std::{path::Path, sync::Arc};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    AGENT_CONFIG_INVALID_SUBJECT, GIT_RECEIVE_ACCEPTED_SUBJECT, GitStorage, GitStorageError,
    RUN_START_SUBJECT,
};

/// Input used to create repository metadata and bare storage.
#[derive(Debug, Clone)]
pub struct CreateRepository {
    /// Owning project.
    pub project_id: ProjectId,
    /// Human-readable name.
    pub name: String,
    /// Fully-qualified default branch.
    pub default_branch: GitRef,
    /// Whether unaffiliated users may clone and fetch.
    pub is_public: bool,
    /// Whether valid pushes may trigger runs.
    pub agent_runs_enabled: bool,
}

/// A durable run request emitted by receive processing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunRequest {
    /// Stable request identifier.
    pub id: RunRequestId,
    /// Exact repository.
    pub repository_id: RepositoryId,
    /// Exact received commit.
    pub commit_sha: CommitSha,
    /// Exact updated ref.
    pub git_ref: GitRef,
    /// Hash of exact `agent.toml` bytes.
    pub config_hash: String,
    /// Receive transaction that accepted the update.
    pub receive_id: ReceiveId,
    /// Command consumed by the run orchestrator.
    pub command: StartRun,
}

/// Committed result of receive processing.
#[derive(Debug, Clone)]
pub struct ReceiveResult {
    /// Receive audit identifier.
    pub receive_id: ReceiveId,
    /// Idempotently created run requests.
    pub run_requests: Vec<RunRequest>,
    /// Number of invalid configuration revisions observed.
    pub invalid_configurations: usize,
}

/// Transactional outbox record.
#[derive(Debug, Clone)]
pub struct OutboxRecord {
    /// Stable publication identifier.
    pub id: EventId,
    /// NATS subject.
    pub subject: String,
    /// JSON payload.
    pub payload: Value,
}

/// `PostgreSQL` forge metadata and receive repository.
#[derive(Clone)]
pub struct PgForgeRepository {
    pool: PgPool,
    storage: Arc<GitStorage>,
    authorizer: Option<Arc<dyn Authorizer>>,
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
    pub fn with_authorizer(mut self, authorizer: Arc<dyn Authorizer>) -> Self {
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
        validate_name(name, "project name must contain 1 to 200 characters")?;
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
            "INSERT INTO projects (id, organization_id, name) VALUES ($1, $2, $3)
             RETURNING id, organization_id, name, created_at",
        )
        .bind(id.as_uuid())
        .bind(organization_id.as_uuid())
        .bind(name)
        .fetch_one(&mut *transaction)
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
        validate_name(name, "project name must contain 1 to 200 characters")?;
        let id = ProjectId::new();
        let row = sqlx::query_as::<_, ProjectRow>(
            "INSERT INTO projects (id, organization_id, name) VALUES ($1, $2, $3)
             RETURNING id, organization_id, name, created_at",
        )
        .bind(id.as_uuid())
        .bind(organization_id.as_uuid())
        .bind(name)
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
        let mut transaction = self.pool.begin().await.map_err(storage)?;
        if let Some(identity) = identity {
            sqlx::query(
                "SELECT set_config('hephaestus.actor_id', $1, true),
                        set_config('hephaestus.request_id', $2, true)",
            )
            .bind(identity.user_id.to_string())
            .bind(identity.request_id.to_string())
            .execute(&mut *transaction)
            .await
            .map_err(storage)?;
        }
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
                "SELECT id, repository_id, commit_sha, git_ref, config_hash,
                        receive_id, agent_id, run_id, command_id
                 FROM run_requests WHERE receive_id = $1 ORDER BY created_at, id",
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
            transaction.commit().await.map_err(storage)?;
            return Ok(ReceiveResult {
                receive_id,
                run_requests,
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
        append_outbox(
            &mut transaction,
            receive_id.as_uuid(),
            GIT_RECEIVE_ACCEPTED_SUBJECT,
            "git.receive.accepted",
            json!({
                "receive_id": receive_id,
                "repository_id": repository.id,
                "principal": principal,
                "updates": updates,
            }),
            now,
        )
        .await?;

        let mut run_requests = Vec::new();
        let mut invalid_configurations = 0;
        for item in inspected {
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
            let (revision_id, agent_id) = persist_revision(
                &mut transaction,
                repository.id,
                repository.project_id,
                receive_id,
                &item.commit,
                &parsed,
                now,
            )
            .await?;
            let Some(config) = parsed.config.as_ref() else {
                invalid_configurations += 1;
                append_outbox(
                    &mut transaction,
                    revision_id.as_uuid(),
                    AGENT_CONFIG_INVALID_SUBJECT,
                    "git.agent_config.invalid",
                    json!({
                        "repository_id": repository.id,
                        "receive_id": receive_id,
                        "commit_sha": item.commit,
                        "git_ref": item.git_ref,
                        "config_hash": parsed.hash,
                        "diagnostics": parsed.diagnostics,
                    }),
                    now,
                )
                .await?;
                continue;
            };
            if repository.agent_runs_enabled && config.triggers.matches(&item.git_ref) {
                let valid_agent_id = agent_id.ok_or(ForgeRepositoryError::InvalidStoredData(
                    "valid revision agent_id",
                ))?;
                if let (Some(identity), Some(authorizer)) = (identity, &self.authorizer) {
                    let object = ObjectRef::new(ObjectType::Agent, valid_agent_id.as_uuid());
                    let decision = authorizer
                        .check(
                            &mut transaction,
                            Subject::User(identity.user_id),
                            Permission::CanExecute,
                            object,
                        )
                        .await
                        .map_err(storage)?;
                    audit_decision(
                        &mut transaction,
                        identity.user_id,
                        Permission::CanExecute,
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
                let request = persist_run_request(
                    &mut transaction,
                    repository.id,
                    receive_id,
                    revision_id,
                    valid_agent_id,
                    identity,
                    &item.git_ref,
                    &item.commit,
                    &parsed.hash,
                    now,
                )
                .await?;
                run_requests.push(request);
            }
        }
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
            invalid_configurations,
        })
    }

    /// Loads unpublished outbox entries.
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
             WHERE published_at IS NULL AND aggregate_type = 'forge'
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
                Ok(InspectedUpdate {
                    git_ref: update.git_ref.clone(),
                    commit: commit.clone(),
                    parsed,
                })
            })
        })
        .collect()
}

async fn persist_revision(
    transaction: &mut Transaction<'_, Postgres>,
    repository_id: RepositoryId,
    project_id: ProjectId,
    receive_id: ReceiveId,
    commit: &CommitSha,
    parsed: &ParsedConfig,
    now: OffsetDateTime,
) -> Result<(AgentConfigRevisionId, Option<AgentId>), ForgeRepositoryError> {
    let revision_id = AgentConfigRevisionId::new();
    let agent_id = if let Some(config) = parsed.config.as_ref() {
        let stored: Uuid = sqlx::query_scalar(
            "INSERT INTO agents (id, project_id, name)
             VALUES ($1, $2, $3)
             ON CONFLICT (project_id, name) DO UPDATE
             SET name = EXCLUDED.name, updated_at = now()
             RETURNING id",
        )
        .bind(AgentId::new().as_uuid())
        .bind(project_id.as_uuid())
        .bind(&config.agent.name)
        .fetch_one(&mut **transaction)
        .await
        .map_err(storage)?;
        Some(AgentId::from_uuid(stored))
    } else {
        None
    };
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
          agent_id, status, config, diagnostics, created_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
         ON CONFLICT (repository_id, commit_sha, config_hash)
         DO UPDATE SET repository_id = EXCLUDED.repository_id
         RETURNING id, agent_id",
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
    .bind(agent_id.map(AgentId::as_uuid))
    .bind(status)
    .bind(config)
    .bind(diagnostics)
    .bind(now)
    .fetch_one(&mut **transaction)
    .await
    .map_err(storage)?;
    Ok((
        AgentConfigRevisionId::from_uuid(row.id),
        row.agent_id.map(AgentId::from_uuid),
    ))
}

#[allow(clippy::too_many_arguments)]
async fn persist_run_request(
    transaction: &mut Transaction<'_, Postgres>,
    repository_id: RepositoryId,
    receive_id: ReceiveId,
    revision_id: AgentConfigRevisionId,
    agent_id: AgentId,
    identity: Option<&AuthenticatedIdentity>,
    git_ref: &GitRef,
    commit: &CommitSha,
    config_hash: &ConfigHash,
    now: OffsetDateTime,
) -> Result<RunRequest, ForgeRepositoryError> {
    let request_id = RunRequestId::new();
    let run_id = RunId::new();
    let command_id = CommandId::new();
    let row = sqlx::query_as::<_, RunRequestRow>(
        "INSERT INTO run_requests
         (id, repository_id, commit_sha, git_ref, config_hash, receive_id,
          config_revision_id, agent_id, run_id, command_id, actor_id, request_id,
          created_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
         ON CONFLICT (
             repository_id, commit_sha, git_ref, config_hash, receive_id, attempt
         )
         DO UPDATE SET repository_id = EXCLUDED.repository_id
         RETURNING id, repository_id, commit_sha, git_ref, config_hash,
                   receive_id, agent_id, run_id, command_id",
    )
    .bind(request_id.as_uuid())
    .bind(repository_id.as_uuid())
    .bind(commit.as_str())
    .bind(git_ref.as_str())
    .bind(config_hash.as_str())
    .bind(receive_id.as_uuid())
    .bind(revision_id.as_uuid())
    .bind(agent_id.as_uuid())
    .bind(run_id.as_uuid())
    .bind(command_id.as_uuid())
    .bind(identity.map(|value| value.user_id.as_uuid()))
    .bind(identity.map(|value| value.request_id.as_uuid()))
    .bind(now)
    .fetch_one(&mut **transaction)
    .await
    .map_err(storage)?;
    let request: RunRequest = row.try_into()?;
    append_outbox(
        transaction,
        request.id.as_uuid(),
        RUN_START_SUBJECT,
        "run.start",
        serde_json::to_value(&request.command).map_err(serialization)?,
        now,
    )
    .await?;
    Ok(request)
}

async fn append_outbox(
    transaction: &mut Transaction<'_, Postgres>,
    aggregate_id: Uuid,
    subject: &str,
    event_type: &str,
    payload: Value,
    occurred_at: OffsetDateTime,
) -> Result<(), ForgeRepositoryError> {
    sqlx::query(
        "INSERT INTO outbox
         (id, aggregate_type, aggregate_id, subject, event_type, payload, occurred_at)
         VALUES ($1, 'forge', $2, $3, $4, $5, $6)",
    )
    .bind(EventId::new().as_uuid())
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
    agent_id: Option<Uuid>,
}

#[derive(FromRow)]
struct RunRequestRow {
    id: Uuid,
    repository_id: Uuid,
    commit_sha: String,
    git_ref: String,
    config_hash: String,
    receive_id: Uuid,
    agent_id: Uuid,
    run_id: Uuid,
    command_id: Uuid,
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
            config_hash: row.config_hash,
            receive_id: ReceiveId::from_uuid(row.receive_id),
            command: StartRun {
                command_id: CommandId::from_uuid(row.command_id),
                run_id: RunId::from_uuid(row.run_id),
                agent_id: AgentId::from_uuid(row.agent_id),
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

/// Durable forge service failure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ForgeRepositoryError {
    /// Repository metadata does not exist.
    #[error("repository {0} was not found")]
    RepositoryNotFound(RepositoryId),
    /// A receive identifier was already used for a different repository.
    #[error("receive identifier {0} conflicts with an existing receive")]
    ReceiveConflict(ReceiveId),
    /// Caller metadata is invalid.
    #[error("invalid forge metadata: {0}")]
    InvalidMetadata(&'static str),
    /// Stored data violates domain invariants.
    #[error("invalid stored forge data in {0}")]
    InvalidStoredData(&'static str),
    /// The authenticated actor lacks the required command permission.
    #[error("forge command is not authorized")]
    AuthorizationDenied,
    /// An authenticated workflow was started without an authorization provider.
    #[error("forge authorization provider is unavailable")]
    AuthorizationUnavailable,
    /// Bare repository storage failed.
    #[error(transparent)]
    GitStorage(#[from] GitStorageError),
    /// Git object inspection failed.
    #[error("Git object inspection failed: {0}")]
    GitInspection(String),
    /// JSON encoding failed.
    #[error("forge serialization failed: {0}")]
    Serialization(#[source] serde_json::Error),
    /// `PostgreSQL` access failed.
    #[error("forge persistence failed: {0}")]
    Storage(#[source] Box<dyn std::error::Error + Send + Sync>),
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
