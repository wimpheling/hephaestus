//! Transactional reusable release publication, project instance import,
//! immutable revision, and repository attachment workflows.

use agent_config::{AgentConfig, NetworkProfile, ParameterDefault, REUSABLE_RELEASE_VERSION};
use async_nats::{HeaderMap, jetstream};
use authz_domain::{AuthorizationDecision, Authorizer, ObjectRef, ObjectType, Permission, Subject};
use authz_postgres::{audit_decision, begin_actor_transaction};
use forge_domain::{ProjectId, RepositoryId};
use identity_domain::AuthenticatedIdentity;
use release_domain::{
    AgentAttachmentId, AgentFamilyId, AgentInstanceId, AgentInstanceRevisionId, AgentKey,
    AgentUpdateId, ArtifactKind, ArtifactPath, BuildRequestId, ContentHash, InstanceName,
    NetworkAccess, ParameterDeclaration, ParameterDocument, ParameterName, ParameterType,
    ParameterValue, RefSelector, ReleaseAgentId, ReleaseArtifactId, ReleaseCommandKey, ReleaseId,
    ReleaseVersion, RuntimePolicy, TriggerPolicy,
};
use run_domain::{RunKind, StartRun};
use runtime_types::{CommandId, RunId};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Transaction};
use std::{collections::BTreeMap, sync::Arc};
use time::OffsetDateTime;
use uuid::Uuid;

const RUN_START_SUBJECT: &str = "hephaestus.run.start";
const RELEASE_EVENT_STREAM: &str = "HEPHAESTUS_RELEASE_EVENTS";

/// Creates the durable stream for informational release lifecycle events.
///
/// Actionable run and instance-trigger subjects are owned by their existing
/// command streams and are intentionally excluded here.
///
/// # Errors
///
/// Returns an error when `JetStream` rejects topology creation.
pub async fn ensure_release_jetstream_topology(
    context: &jetstream::Context,
) -> Result<(), ReleaseOutboxPublishError> {
    use jetstream::stream::{Config, RetentionPolicy, StorageType};

    context
        .get_or_create_stream(Config {
            name: RELEASE_EVENT_STREAM.to_owned(),
            subjects: vec![
                String::from("hephaestus.build.completed.v1"),
                String::from("hephaestus.build.failed.v1"),
                String::from("hephaestus.release.>"),
                String::from("hephaestus.agent_instance.>"),
                String::from("hephaestus.agent_update.>"),
            ],
            retention: RetentionPolicy::Limits,
            storage: StorageType::File,
            ..Default::default()
        })
        .await
        .map_err(|error| ReleaseOutboxPublishError::JetStream(error.to_string()))?;
    Ok(())
}

/// Publishes release-owned transactional outbox records to `JetStream`.
#[derive(Clone)]
pub struct ReleaseOutboxPublisher {
    context: jetstream::Context,
    pool: PgPool,
}

impl ReleaseOutboxPublisher {
    /// Creates a publisher for release-owned records.
    #[must_use]
    pub const fn new(context: jetstream::Context, pool: PgPool) -> Self {
        Self { context, pool }
    }

    /// Publishes and marks up to `limit` pending records.
    ///
    /// # Errors
    ///
    /// Returns after recording the first database or publication failure.
    pub async fn publish_pending(&self, limit: i64) -> Result<usize, ReleaseOutboxPublishError> {
        let rows = sqlx::query_as::<_, ReleaseOutboxRow>(
            "SELECT id, subject, payload FROM outbox
             WHERE published_at IS NULL AND aggregate_type = 'release'
             ORDER BY occurred_at, id LIMIT $1",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        let count = rows.len();
        for row in rows {
            let payload = serde_json::to_vec(&row.payload)?;
            let mut headers = HeaderMap::new();
            headers.insert("Nats-Msg-Id", row.id.to_string());
            let publication = self
                .context
                .publish_with_headers(row.subject, headers, payload.into())
                .await;
            let result = match publication {
                Ok(acknowledgement) => acknowledgement.await.map_err(|error| error.to_string()),
                Err(error) => Err(error.to_string()),
            };
            match result {
                Ok(_) => {
                    sqlx::query(
                        "UPDATE outbox
                         SET published_at = now(), attempts = attempts + 1,
                             last_error = NULL
                         WHERE id = $1",
                    )
                    .bind(row.id)
                    .execute(&self.pool)
                    .await?;
                }
                Err(error) => {
                    sqlx::query(
                        "UPDATE outbox
                         SET attempts = attempts + 1, last_error = $2
                         WHERE id = $1",
                    )
                    .bind(row.id)
                    .bind(&error)
                    .execute(&self.pool)
                    .await?;
                    return Err(ReleaseOutboxPublishError::JetStream(error));
                }
            }
        }
        Ok(count)
    }
}

#[derive(sqlx::FromRow)]
struct ReleaseOutboxRow {
    id: Uuid,
    subject: String,
    payload: Value,
}

/// Release outbox database, serialization, or publication failure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ReleaseOutboxPublishError {
    /// `PostgreSQL` access failed.
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    /// Command serialization failed.
    #[error(transparent)]
    Serialization(#[from] serde_json::Error),
    /// `JetStream` rejected publication.
    #[error("JetStream publication failed: {0}")]
    JetStream(String),
}

/// One already safely imported immutable artifact.
#[derive(Debug, Clone)]
pub struct ReleaseArtifactInput {
    /// Stable artifact identity.
    pub id: ReleaseArtifactId,
    /// Normalized path.
    pub path: ArtifactPath,
    /// Artifact kind.
    pub kind: ArtifactKind,
    /// Unix permission bits.
    pub mode: u16,
    /// Exact SHA-256 content hash.
    pub content_hash: ContentHash,
    /// Byte length.
    pub size_bytes: u64,
    /// Bounded media type.
    pub media_type: String,
    /// Opaque canonical storage key.
    pub storage_key: Uuid,
}

/// Trusted worker command that turns a complete safely imported build into a
/// draft release.
#[derive(Debug, Clone)]
pub struct CompleteBuild {
    /// Idempotency identity.
    pub command_key: ReleaseCommandKey,
    /// Exact completed build.
    pub build_request_id: BuildRequestId,
    /// Stable draft release.
    pub release_id: ReleaseId,
    /// Repository-scoped release version.
    pub version: ReleaseVersion,
    /// Stable release export identity.
    pub release_agent_id: ReleaseAgentId,
    /// Complete immutable artifacts.
    pub artifacts: Vec<ReleaseArtifactInput>,
}

/// Project import command.
#[derive(Debug, Clone)]
pub struct ImportAgent {
    /// Idempotency identity.
    pub command_key: ReleaseCommandKey,
    /// Stable product-level instance.
    pub instance_id: AgentInstanceId,
    /// Stable initial revision.
    pub revision_id: AgentInstanceRevisionId,
    /// Consuming project.
    pub project_id: ProjectId,
    /// Published reusable export.
    pub release_agent_id: ReleaseAgentId,
    /// Project-scoped name.
    pub name: InstanceName,
    /// Explicit typed values.
    pub parameters: BTreeMap<ParameterName, ParameterValue>,
    /// Project resource/network restriction.
    pub selected_policy: RuntimePolicy,
    /// Current platform ceiling.
    pub platform_policy: RuntimePolicy,
    /// Current platform policy version.
    pub platform_policy_version: String,
}

/// Repository/ref attachment command.
#[derive(Debug, Clone)]
pub struct CreateAttachment {
    /// Idempotency identity.
    pub command_key: ReleaseCommandKey,
    /// Stable attachment.
    pub attachment_id: AgentAttachmentId,
    /// Parent instance.
    pub instance_id: AgentInstanceId,
    /// Target repository.
    pub repository_id: RepositoryId,
    /// Exact or prefix ref selection.
    pub ref_selector: RefSelector,
    /// Trigger behavior.
    pub trigger_policy: TriggerPolicy,
}

/// Enables or disables one exact attachment without changing provenance.
#[derive(Debug, Clone)]
pub struct SetAttachmentEnabled {
    /// Idempotency identity.
    pub command_key: ReleaseCommandKey,
    /// Exact attachment.
    pub attachment_id: AgentAttachmentId,
    /// Desired trigger state.
    pub enabled: bool,
}

/// Tombstones one exact attachment.
#[derive(Debug, Clone)]
pub struct RemoveAttachment {
    /// Idempotency identity.
    pub command_key: ReleaseCommandKey,
    /// Exact attachment.
    pub attachment_id: AgentAttachmentId,
}

/// Creates and activates a new immutable parameter/resource revision.
#[derive(Debug, Clone)]
pub struct ReviseInstance {
    /// Idempotency identity.
    pub command_key: ReleaseCommandKey,
    /// Parent project instance.
    pub instance_id: AgentInstanceId,
    /// Compare-and-swap expected active revision.
    pub expected_revision_id: AgentInstanceRevisionId,
    /// Stable new immutable revision.
    pub new_revision_id: AgentInstanceRevisionId,
    /// Complete explicit typed parameter overrides.
    pub parameters: BTreeMap<ParameterName, ParameterValue>,
    /// New project restriction within release bounds.
    pub selected_policy: RuntimePolicy,
    /// Current platform ceiling.
    pub platform_policy: RuntimePolicy,
    /// Current platform policy version.
    pub platform_policy_version: String,
}

/// Creates a fully resolved candidate release update and closes the normal run
/// gate only when the candidate is runnable and state-compatible.
#[derive(Debug, Clone)]
pub struct CreateInstanceUpdate {
    /// Idempotency identity.
    pub command_key: ReleaseCommandKey,
    /// Stable update identity delivered to the hook.
    pub update_id: AgentUpdateId,
    /// Parent instance.
    pub instance_id: AgentInstanceId,
    /// Compare-and-swap current revision.
    pub expected_revision_id: AgentInstanceRevisionId,
    /// Stable candidate revision.
    pub candidate_revision_id: AgentInstanceRevisionId,
    /// Published candidate export in the exact same family.
    pub candidate_release_agent_id: ReleaseAgentId,
    /// Candidate typed parameters.
    pub parameters: BTreeMap<ParameterName, ParameterValue>,
    /// Candidate project restriction.
    pub selected_policy: RuntimePolicy,
    /// Current platform ceiling.
    pub platform_policy: RuntimePolicy,
    /// Current platform policy version.
    pub platform_policy_version: String,
}

/// Agent update-hook terminal result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateHookResult {
    /// Exit zero: agent committed its state migration.
    Committed,
    /// Explicit nonzero exit: agent reports it rolled its own state back.
    Rejected(i32),
    /// Signal, timeout, VM loss, or protocol uncertainty.
    Uncertain,
}

/// Durable platform decision after a hook terminal result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateDecision {
    /// Candidate revision became active and the gate reopened.
    Activated,
    /// Current revision remains active and the gate reopened.
    AgentRejected,
    /// Current revision remains selected but the instance is paused.
    CompatibilityUnknown,
    /// Hook committed but activation needs operator recovery.
    ActivationRecovery,
}

/// Starts the isolated update hook after the pre-gate run set drains.
#[derive(Debug, Clone)]
pub struct BeginUpdateHook {
    /// Idempotency identity.
    pub command_key: ReleaseCommandKey,
    /// Exact update.
    pub update_id: AgentUpdateId,
    /// Stable special update run created atomically with hook admission.
    pub hook_run_id: RunId,
}

/// Explicit operator choice for an update paused at a recovery boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateRecoveryAction {
    /// Re-run an idempotent hook with the same stable update identity.
    RetryHook,
    /// Keep the current revision and accept responsibility for agent-owned state.
    RejectCandidate,
    /// Finish activation after the hook's durable success commit point.
    ResumeActivation,
}

impl UpdateRecoveryAction {
    const fn operation(self) -> &'static str {
        match self {
            Self::RetryHook => "recover_update_retry",
            Self::RejectCandidate => "recover_update_reject",
            Self::ResumeActivation => "recover_update_resume",
        }
    }
}

/// Authorized, idempotent recovery command for one paused update.
#[derive(Debug, Clone)]
pub struct RecoverInstanceUpdate {
    /// Idempotency identity.
    pub command_key: ReleaseCommandKey,
    /// Exact update retaining its stable guest-visible identity.
    pub update_id: AgentUpdateId,
    /// Explicit recovery choice.
    pub action: UpdateRecoveryAction,
}

/// Durable result of an explicit update recovery command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateRecoveryDecision {
    /// The same update may start a new hook attempt after normal-work drain.
    HookRetryScheduled,
    /// The prior revision remains selected and its run gate is open.
    CandidateRejected,
    /// The hook-committed candidate is active and its run gate is open.
    CandidateActivated,
}

/// PostgreSQL-backed release and instance command service.
pub struct ReleaseService {
    pool: PgPool,
    authorizer: Arc<dyn Authorizer>,
}

impl ReleaseService {
    /// Creates the service.
    #[must_use]
    pub fn new(pool: PgPool, authorizer: Arc<dyn Authorizer>) -> Self {
        Self { pool, authorizer }
    }

    /// Freezes complete build provenance into a draft release. This trusted
    /// worker operation accepts only artifact metadata from a completed safe
    /// importer; it never receives repository-controlled storage paths.
    ///
    /// # Errors
    ///
    /// Returns a typed failure for incomplete builds, invalid configuration or
    /// artifacts, conflicting publication identity, or database failure.
    // The ordered inserts document the one-transaction draft invariant.
    #[allow(clippy::too_many_lines)]
    #[tracing::instrument(
        skip_all,
        fields(
            build_request_id = %command.build_request_id,
            release_id = %command.release_id,
            release_agent_id = %command.release_agent_id
        )
    )]
    pub async fn complete_build(
        &self,
        command: CompleteBuild,
    ) -> Result<ReleaseId, ReleaseServiceError> {
        if command.artifacts.is_empty() {
            return Err(ReleaseServiceError::IncompleteArtifacts);
        }
        let mut tx = self.pool.begin().await?;
        if let Some(id) = existing_command(&mut tx, command.command_key, "complete_build").await? {
            tx.commit().await?;
            return Ok(ReleaseId::from_uuid(id.0));
        }
        let build: BuildRow = sqlx::query_as(
            "SELECT repository_id, source_commit, source_ref,
                    build_definition_hash, state
             FROM build_requests WHERE id = $1 FOR UPDATE",
        )
        .bind(command.build_request_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(ReleaseServiceError::Unavailable)?;
        if !matches!(build.state.as_str(), "importing" | "succeeded") {
            return Err(ReleaseServiceError::BuildNotImporting);
        }
        let config_row: (Value, Option<String>) = sqlx::query_as(
            "SELECT config, normalized_config_hash
             FROM agent_config_revisions
             WHERE repository_id = $1 AND commit_sha = $2
               AND schema_version = $3 AND status = 'valid'
             ORDER BY created_at DESC LIMIT 1",
        )
        .bind(build.repository_id)
        .bind(&build.source_commit)
        .bind(i32::try_from(REUSABLE_RELEASE_VERSION).unwrap_or(i32::MAX))
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(ReleaseServiceError::ReusableConfigurationMissing)?;
        let config: AgentConfig = serde_json::from_value(config_row.0.clone())?;
        let agent_key = AgentKey::parse(
            config
                .agent
                .key
                .clone()
                .ok_or(ReleaseServiceError::ReusableConfigurationMissing)?,
        )?;
        let family_id = AgentFamilyId::new();
        let stored_family: Uuid = sqlx::query_scalar(
            "INSERT INTO agent_families (id, repository_id, agent_key)
             VALUES ($1, $2, $3)
             ON CONFLICT (repository_id, agent_key)
             DO UPDATE SET agent_key = EXCLUDED.agent_key
             RETURNING id",
        )
        .bind(family_id.as_uuid())
        .bind(build.repository_id)
        .bind(agent_key.as_str())
        .fetch_one(&mut *tx)
        .await?;
        let family_id = AgentFamilyId::from_uuid(stored_family);
        let manifest_hash = artifact_manifest_hash(&command.artifacts);
        let configuration_hash = decode_hash(
            config_row
                .1
                .as_deref()
                .ok_or(ReleaseServiceError::ReusableConfigurationMissing)?,
        )?;
        sqlx::query(
            "INSERT INTO releases
             (id, repository_id, version, source_commit, source_ref,
              build_request_id, build_definition_hash, configuration,
              configuration_hash, manifest_hash, state)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, 'draft')",
        )
        .bind(command.release_id.as_uuid())
        .bind(build.repository_id)
        .bind(command.version.as_str())
        .bind(&build.source_commit)
        .bind(&build.source_ref)
        .bind(command.build_request_id.as_uuid())
        .bind(&build.build_definition_hash)
        .bind(&config_row.0)
        .bind(configuration_hash.as_slice())
        .bind(manifest_hash.as_bytes().as_slice())
        .execute(&mut *tx)
        .await?;
        for artifact in &command.artifacts {
            validate_artifact(artifact)?;
            sqlx::query(
                "INSERT INTO release_artifacts
                 (id, release_id, path, kind, mode, content_hash, size_bytes,
                  media_type, storage_key)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
            )
            .bind(artifact.id.as_uuid())
            .bind(command.release_id.as_uuid())
            .bind(artifact.path.as_str())
            .bind(artifact_kind_name(artifact.kind))
            .bind(i32::from(artifact.mode))
            .bind(artifact.content_hash.as_bytes().as_slice())
            .bind(
                i64::try_from(artifact.size_bytes)
                    .map_err(|_| ReleaseServiceError::InvalidArtifact)?,
            )
            .bind(&artifact.media_type)
            .bind(artifact.storage_key)
            .execute(&mut *tx)
            .await?;
        }
        let policy = runtime_policy(&config);
        let parameters = parameter_schema(&config)?;
        let executable = ArtifactPath::parse(config.guest.command.clone())?;
        let working_directory = ArtifactPath::parse(config.guest.working_directory.clone())?;
        let contract = json!({
            "executable": executable,
            "arguments": config.guest.arguments,
            "working_directory": working_directory,
            "root_image_digest": config.root_image.reference,
            "requires_state": config.state_volume.enabled,
            "policy_ceiling": policy,
            "workspace": {
                "source": "/workspace/repo",
                "work": "/workspace/work",
                "release": "/release",
                "state": "/var/lib/hephaestus",
                "parameters": "/run/hephaestus/parameters.json"
            }
        });
        let contract_bytes = serde_json::to_vec(&contract)?;
        let contract_hash: [u8; 32] = Sha256::digest(&contract_bytes).into();
        sqlx::query(
            "INSERT INTO release_agents
             (id, release_id, family_id, agent_key, display_name,
              runtime_contract, runtime_contract_hash, parameter_schema,
              secret_slot_schema, requires_state, update_hook)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
        )
        .bind(command.release_agent_id.as_uuid())
        .bind(command.release_id.as_uuid())
        .bind(family_id.as_uuid())
        .bind(agent_key.as_str())
        .bind(&config.agent.name)
        .bind(contract)
        .bind(contract_hash.as_slice())
        .bind(serde_json::to_value(&parameters)?)
        .bind(serde_json::to_value(&config.secret_slots)?)
        .bind(config.state_volume.enabled)
        .bind(
            config
                .update_hook
                .as_ref()
                .map(serde_json::to_value)
                .transpose()?,
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE build_requests SET state = 'succeeded', completed_at = now()
             WHERE id = $1",
        )
        .bind(command.build_request_id.as_uuid())
        .execute(&mut *tx)
        .await?;
        record_command(
            &mut tx,
            command.command_key,
            "complete_build",
            command.release_id.as_uuid(),
            Some(command.release_agent_id.as_uuid()),
            None,
        )
        .await?;
        append_event(
            &mut tx,
            command.release_id.as_uuid(),
            "hephaestus.build.completed.v1",
            "build.completed.v1",
            json!({
                "schema_version": 1,
                "build_request_id": command.build_request_id,
                "release_id": command.release_id,
                "release_agent_id": command.release_agent_id,
                "manifest_hash": hex_hash(manifest_hash.as_bytes()),
            }),
        )
        .await?;
        tx.commit().await?;
        Ok(command.release_id)
    }

    /// Explicitly publishes and freezes one complete draft release.
    ///
    /// # Errors
    ///
    /// Fails for denial, missing/incomplete draft, idempotency conflict, or
    /// database failure.
    #[tracing::instrument(
        skip_all,
        fields(actor_id = %identity.user_id, request_id = %identity.request_id, %release_id)
    )]
    pub async fn publish(
        &self,
        identity: &AuthenticatedIdentity,
        command_key: ReleaseCommandKey,
        release_id: ReleaseId,
    ) -> Result<(), ReleaseServiceError> {
        let mut tx = begin_actor_transaction(&self.pool, identity).await?;
        self.require(
            &mut tx,
            identity,
            Permission::CanPublish,
            ObjectRef::new(ObjectType::Release, release_id.as_uuid()),
        )
        .await?;
        if existing_command(&mut tx, command_key, "publish")
            .await?
            .is_some()
        {
            tx.commit().await?;
            return Ok(());
        }
        let changed = sqlx::query(
            "UPDATE releases SET state = 'published',
                    publication_actor_id = $2, published_at = now()
             WHERE id = $1 AND state = 'draft'
               AND EXISTS (
                    SELECT 1 FROM release_artifacts WHERE release_id = $1
               )
               AND EXISTS (
                    SELECT 1 FROM release_agents WHERE release_id = $1
               )",
        )
        .bind(release_id.as_uuid())
        .bind(identity.user_id.as_uuid())
        .execute(&mut *tx)
        .await?;
        if changed.rows_affected() != 1 {
            return Err(ReleaseServiceError::Unavailable);
        }
        record_command(
            &mut tx,
            command_key,
            "publish",
            release_id.as_uuid(),
            None,
            Some(identity),
        )
        .await?;
        append_event(
            &mut tx,
            release_id.as_uuid(),
            "hephaestus.release.published.v1",
            "release.published.v1",
            json!({"schema_version": 1, "release_id": release_id}),
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Revokes a published release without deleting immutable provenance.
    ///
    /// Historical instances, revisions, runs, results, and artifacts retain
    /// their exact foreign-key targets, while new imports and guest starts
    /// reject the no-longer-published release.
    ///
    /// # Errors
    ///
    /// Fails for denial, a non-published release, idempotency conflict, or a
    /// database error.
    #[tracing::instrument(
        skip_all,
        fields(actor_id = %identity.user_id, request_id = %identity.request_id, %release_id)
    )]
    pub async fn revoke(
        &self,
        identity: &AuthenticatedIdentity,
        command_key: ReleaseCommandKey,
        release_id: ReleaseId,
    ) -> Result<(), ReleaseServiceError> {
        let mut tx = begin_actor_transaction(&self.pool, identity).await?;
        self.require(
            &mut tx,
            identity,
            Permission::CanRevoke,
            ObjectRef::new(ObjectType::Release, release_id.as_uuid()),
        )
        .await?;
        if existing_command(&mut tx, command_key, "revoke")
            .await?
            .is_some()
        {
            tx.commit().await?;
            return Ok(());
        }
        let changed = sqlx::query(
            "UPDATE releases
             SET state = 'revoked', revoked_at = now()
             WHERE id = $1 AND state = 'published'",
        )
        .bind(release_id.as_uuid())
        .execute(&mut *tx)
        .await?;
        if changed.rows_affected() != 1 {
            return Err(ReleaseServiceError::Unavailable);
        }
        record_command(
            &mut tx,
            command_key,
            "revoke",
            release_id.as_uuid(),
            None,
            Some(identity),
        )
        .await?;
        append_event(
            &mut tx,
            release_id.as_uuid(),
            "hephaestus.release.revoked.v1",
            "release.revoked.v1",
            json!({"release_id": release_id}),
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Imports a published release export as a project-owned instance and
    /// atomically activates its first immutable revision.
    ///
    /// Required unresolved secret slots make the revision visibly unrunnable;
    /// no secret value or tenant secret identifier is present in the release.
    ///
    /// # Errors
    ///
    /// Fails for either project-side or source-side denial, invalid parameters
    /// or policy, unpublished release, idempotency conflict, or database error.
    // Instance, optional state-volume metadata, and revision must commit as one.
    #[allow(clippy::too_many_lines)]
    #[tracing::instrument(
        skip_all,
        fields(
            actor_id = %identity.user_id,
            request_id = %identity.request_id,
            instance_id = %command.instance_id,
            release_agent_id = %command.release_agent_id,
            project_id = %command.project_id
        )
    )]
    pub async fn import_agent(
        &self,
        identity: &AuthenticatedIdentity,
        command: ImportAgent,
    ) -> Result<AgentInstanceId, ReleaseServiceError> {
        let mut tx = begin_actor_transaction(&self.pool, identity).await?;
        self.require(
            &mut tx,
            identity,
            Permission::CanManage,
            ObjectRef::new(ObjectType::Project, command.project_id.as_uuid()),
        )
        .await?;
        self.require(
            &mut tx,
            identity,
            Permission::CanUse,
            ObjectRef::new(ObjectType::ReleaseAgent, command.release_agent_id.as_uuid()),
        )
        .await?;
        if let Some(id) = existing_command(&mut tx, command.command_key, "import_agent").await? {
            tx.commit().await?;
            return Ok(AgentInstanceId::from_uuid(id.0));
        }
        let release: ReleaseAgentRow = sqlx::query_as(
            "SELECT agent.family_id, agent.parameter_schema,
                    agent.secret_slot_schema, agent.runtime_contract,
                    agent.requires_state
             FROM release_agents AS agent
             JOIN releases ON releases.id = agent.release_id
             WHERE agent.id = $1 AND releases.state = 'published'",
        )
        .bind(command.release_agent_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(ReleaseServiceError::Unavailable)?;
        let declarations: Vec<ParameterDeclaration> =
            serde_json::from_value(release.parameter_schema)?;
        let parameters = ParameterDocument::resolve(&declarations, &command.parameters)
            .map_err(ReleaseServiceError::InvalidParameters)?;
        let release_policy = policy_from_contract(&release.runtime_contract)?;
        let effective_policy = RuntimePolicy::resolve(
            &release_policy,
            &command.selected_policy,
            &command.platform_policy,
        )?;
        let required_slots = release
            .secret_slot_schema
            .as_array()
            .ok_or(ReleaseServiceError::InvalidStoredData)?
            .iter()
            .filter(|slot| slot.get("required").and_then(Value::as_bool) == Some(true))
            .filter_map(|slot| slot.get("key").and_then(Value::as_str))
            .collect::<Vec<_>>();
        let runnable = required_slots.is_empty();
        let diagnostics = required_slots
            .into_iter()
            .map(|slot| {
                json!({
                    "code": "required_secret_binding_missing",
                    "field": format!("secret_slots.{slot}")
                })
            })
            .collect::<Vec<_>>();
        let state_volume_id = release.requires_state.then(Uuid::new_v4);
        sqlx::query(
            "INSERT INTO agent_instances
             (id, project_id, family_id, name, state, active_revision_id,
              state_volume_id, created_by)
             VALUES ($1, $2, $3, $4, 'active', NULL, $5, $6)",
        )
        .bind(command.instance_id.as_uuid())
        .bind(command.project_id.as_uuid())
        .bind(release.family_id)
        .bind(command.name.as_str())
        .bind(state_volume_id)
        .bind(identity.user_id.as_uuid())
        .execute(&mut *tx)
        .await?;
        if let Some(volume_id) = state_volume_id {
            sqlx::query(
                "INSERT INTO agent_instance_state_volumes
                 (id, instance_id, state, capacity_bytes)
                 VALUES ($1, $2, 'uninitialized', 1073741824)",
            )
            .bind(volume_id)
            .bind(command.instance_id.as_uuid())
            .execute(&mut *tx)
            .await?;
        }
        insert_revision(
            &mut tx,
            command.revision_id,
            command.instance_id,
            command.release_agent_id,
            &parameters,
            &command.selected_policy,
            &effective_policy,
            &command.platform_policy_version,
            runnable,
            &diagnostics,
            identity,
        )
        .await?;
        sqlx::query(
            "UPDATE agent_instances
             SET active_revision_id = $2, updated_at = now(), version = version + 1
             WHERE id = $1 AND active_revision_id IS NULL",
        )
        .bind(command.instance_id.as_uuid())
        .bind(command.revision_id.as_uuid())
        .execute(&mut *tx)
        .await?;
        record_command(
            &mut tx,
            command.command_key,
            "import_agent",
            command.instance_id.as_uuid(),
            Some(command.revision_id.as_uuid()),
            Some(identity),
        )
        .await?;
        append_instance_event(
            &mut tx,
            command.instance_id,
            Some(command.revision_id),
            "instance.created",
            identity,
            json!({"runnable": runnable}),
        )
        .await?;
        append_event(
            &mut tx,
            command.instance_id.as_uuid(),
            "hephaestus.agent_instance.created.v1",
            "agent_instance.created.v1",
            json!({
                "schema_version": 1,
                "instance_id": command.instance_id,
                "revision_id": command.revision_id,
                "release_agent_id": command.release_agent_id,
                "project_id": command.project_id,
                "runnable": runnable,
            }),
        )
        .await?;
        tx.commit().await?;
        Ok(command.instance_id)
    }

    /// Creates an exact project-bound attachment. Trigger policy lives on this
    /// record, not on the release's source branch.
    ///
    /// # Errors
    ///
    /// Fails for either instance or repository denial, cross-project target,
    /// idempotency conflict, or database failure.
    #[tracing::instrument(
        skip_all,
        fields(
            actor_id = %identity.user_id,
            request_id = %identity.request_id,
            attachment_id = %command.attachment_id,
            instance_id = %command.instance_id,
            repository_id = %command.repository_id
        )
    )]
    pub async fn create_attachment(
        &self,
        identity: &AuthenticatedIdentity,
        command: CreateAttachment,
    ) -> Result<AgentAttachmentId, ReleaseServiceError> {
        let mut tx = begin_actor_transaction(&self.pool, identity).await?;
        self.require(
            &mut tx,
            identity,
            Permission::CanManage,
            ObjectRef::new(ObjectType::AgentInstance, command.instance_id.as_uuid()),
        )
        .await?;
        self.require(
            &mut tx,
            identity,
            Permission::CanWrite,
            ObjectRef::new(ObjectType::Repository, command.repository_id.as_uuid()),
        )
        .await?;
        if let Some(id) =
            existing_command(&mut tx, command.command_key, "create_attachment").await?
        {
            tx.commit().await?;
            return Ok(AgentAttachmentId::from_uuid(id.0));
        }
        let project_id: Uuid =
            sqlx::query_scalar("SELECT project_id FROM agent_instances WHERE id = $1")
                .bind(command.instance_id.as_uuid())
                .fetch_optional(&mut *tx)
                .await?
                .ok_or(ReleaseServiceError::Unavailable)?;
        sqlx::query(
            "INSERT INTO agent_attachments
             (id, instance_id, project_id, repository_id, ref_selector,
              trigger_policy, enabled, created_by)
             VALUES ($1, $2, $3, $4, $5, $6, true, $7)",
        )
        .bind(command.attachment_id.as_uuid())
        .bind(command.instance_id.as_uuid())
        .bind(project_id)
        .bind(command.repository_id.as_uuid())
        .bind(ref_selector_string(&command.ref_selector))
        .bind(trigger_policy_name(command.trigger_policy))
        .bind(identity.user_id.as_uuid())
        .execute(&mut *tx)
        .await?;
        record_command(
            &mut tx,
            command.command_key,
            "create_attachment",
            command.attachment_id.as_uuid(),
            Some(command.instance_id.as_uuid()),
            Some(identity),
        )
        .await?;
        append_instance_event(
            &mut tx,
            command.instance_id,
            None,
            "attachment.created",
            identity,
            json!({
                "attachment_id": command.attachment_id,
                "repository_id": command.repository_id,
            }),
        )
        .await?;
        append_event(
            &mut tx,
            command.attachment_id.as_uuid(),
            "hephaestus.agent_instance.attachment_changed.v1",
            "agent_instance.attachment_changed.v1",
            json!({
                "instance_id": command.instance_id,
                "attachment_id": command.attachment_id,
                "repository_id": command.repository_id,
                "action": "created",
                "enabled": true,
            }),
        )
        .await?;
        tx.commit().await?;
        Ok(command.attachment_id)
    }

    /// Enables or disables future triggers for an attachment.
    ///
    /// # Errors
    ///
    /// Fails for denial, a removed/missing attachment, idempotency conflict, or
    /// database failure.
    #[tracing::instrument(
        skip_all,
        fields(
            actor_id = %identity.user_id,
            request_id = %identity.request_id,
            attachment_id = %command.attachment_id
        )
    )]
    pub async fn set_attachment_enabled(
        &self,
        identity: &AuthenticatedIdentity,
        command: SetAttachmentEnabled,
    ) -> Result<(), ReleaseServiceError> {
        let mut tx = begin_actor_transaction(&self.pool, identity).await?;
        self.require(
            &mut tx,
            identity,
            Permission::CanManage,
            ObjectRef::new(ObjectType::AgentAttachment, command.attachment_id.as_uuid()),
        )
        .await?;
        if existing_command(&mut tx, command.command_key, "set_attachment_enabled")
            .await?
            .is_some()
        {
            tx.commit().await?;
            return Ok(());
        }
        let instance_id: Uuid = sqlx::query_scalar(
            "UPDATE agent_attachments
             SET enabled = $2, updated_at = now()
             WHERE id = $1 AND removed_at IS NULL
             RETURNING instance_id",
        )
        .bind(command.attachment_id.as_uuid())
        .bind(command.enabled)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(ReleaseServiceError::Unavailable)?;
        record_command(
            &mut tx,
            command.command_key,
            "set_attachment_enabled",
            command.attachment_id.as_uuid(),
            Some(instance_id),
            Some(identity),
        )
        .await?;
        append_instance_event(
            &mut tx,
            AgentInstanceId::from_uuid(instance_id),
            None,
            if command.enabled {
                "attachment.enabled"
            } else {
                "attachment.disabled"
            },
            identity,
            json!({"attachment_id": command.attachment_id}),
        )
        .await?;
        append_event(
            &mut tx,
            command.attachment_id.as_uuid(),
            "hephaestus.agent_instance.attachment_changed.v1",
            "agent_instance.attachment_changed.v1",
            json!({
                "instance_id": instance_id,
                "attachment_id": command.attachment_id,
                "action": if command.enabled { "enabled" } else { "disabled" },
                "enabled": command.enabled,
            }),
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Tombstones an attachment while preserving historical run references.
    ///
    /// # Errors
    ///
    /// Fails for denial, a missing attachment, idempotency conflict, or
    /// database failure.
    #[tracing::instrument(
        skip_all,
        fields(
            actor_id = %identity.user_id,
            request_id = %identity.request_id,
            attachment_id = %command.attachment_id
        )
    )]
    pub async fn remove_attachment(
        &self,
        identity: &AuthenticatedIdentity,
        command: RemoveAttachment,
    ) -> Result<(), ReleaseServiceError> {
        let mut tx = begin_actor_transaction(&self.pool, identity).await?;
        self.require(
            &mut tx,
            identity,
            Permission::CanManage,
            ObjectRef::new(ObjectType::AgentAttachment, command.attachment_id.as_uuid()),
        )
        .await?;
        if existing_command(&mut tx, command.command_key, "remove_attachment")
            .await?
            .is_some()
        {
            tx.commit().await?;
            return Ok(());
        }
        let instance_id: Uuid = sqlx::query_scalar(
            "UPDATE agent_attachments
             SET enabled = false, removed_at = COALESCE(removed_at, now()),
                 updated_at = now()
             WHERE id = $1
             RETURNING instance_id",
        )
        .bind(command.attachment_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(ReleaseServiceError::Unavailable)?;
        record_command(
            &mut tx,
            command.command_key,
            "remove_attachment",
            command.attachment_id.as_uuid(),
            Some(instance_id),
            Some(identity),
        )
        .await?;
        append_instance_event(
            &mut tx,
            AgentInstanceId::from_uuid(instance_id),
            None,
            "attachment.removed",
            identity,
            json!({"attachment_id": command.attachment_id}),
        )
        .await?;
        append_event(
            &mut tx,
            command.attachment_id.as_uuid(),
            "hephaestus.agent_instance.attachment_changed.v1",
            "agent_instance.attachment_changed.v1",
            json!({
                "instance_id": instance_id,
                "attachment_id": command.attachment_id,
                "action": "removed",
                "enabled": false,
            }),
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Validates, creates, and CAS-activates a new immutable instance
    /// revision. Existing secret bindings are cloned to new revision-bound
    /// identities only while their live imports remain bindable.
    ///
    /// # Errors
    ///
    /// Fails for denial, stale active revision, invalid typed parameters or
    /// policy, revoked secret authority, idempotency conflict, or database
    /// failure.
    #[allow(clippy::too_many_lines)]
    #[tracing::instrument(
        skip_all,
        fields(
            actor_id = %identity.user_id,
            request_id = %identity.request_id,
            instance_id = %command.instance_id,
            revision_id = %command.new_revision_id
        )
    )]
    pub async fn revise_instance(
        &self,
        identity: &AuthenticatedIdentity,
        command: ReviseInstance,
    ) -> Result<AgentInstanceRevisionId, ReleaseServiceError> {
        let mut tx = begin_actor_transaction(&self.pool, identity).await?;
        self.require(
            &mut tx,
            identity,
            Permission::CanManage,
            ObjectRef::new(ObjectType::AgentInstance, command.instance_id.as_uuid()),
        )
        .await?;
        if let Some((_instance, revision)) =
            existing_command(&mut tx, command.command_key, "revise_instance").await?
        {
            tx.commit().await?;
            return Ok(AgentInstanceRevisionId::from_uuid(
                revision.ok_or(ReleaseServiceError::InvalidStoredData)?,
            ));
        }
        let current: RevisionUpdateRow = sqlx::query_as(
            "SELECT instance.active_revision_id, revision.release_agent_id,
                    revision.secret_bindings, agent.parameter_schema,
                    agent.secret_slot_schema, agent.runtime_contract
             FROM agent_instances AS instance
             JOIN agent_instance_revisions AS revision
               ON revision.id = instance.active_revision_id
             JOIN release_agents AS agent
               ON agent.id = revision.release_agent_id
             JOIN releases AS release ON release.id = agent.release_id
             WHERE instance.id = $1
               AND instance.state IN ('active', 'update_rejected')
               AND release.state = 'published'
             FOR UPDATE OF instance",
        )
        .bind(command.instance_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(ReleaseServiceError::Unavailable)?;
        if current.active_revision_id != Some(command.expected_revision_id.as_uuid()) {
            return Err(ReleaseServiceError::StaleInstanceRevision);
        }
        let declarations: Vec<ParameterDeclaration> =
            serde_json::from_value(current.parameter_schema)?;
        let parameters = ParameterDocument::resolve(&declarations, &command.parameters)
            .map_err(ReleaseServiceError::InvalidParameters)?;
        let release_policy = policy_from_contract(&current.runtime_contract)?;
        let effective = RuntimePolicy::resolve(
            &release_policy,
            &command.selected_policy,
            &command.platform_policy,
        )?;
        let carried: Vec<RevisionBindingRow> = sqlx::query_as(
            "SELECT binding.import_id, binding.slot_key,
                    binding.delivery_mode, binding.phases,
                    binding.attachment_ids, binding.destinations,
                    binding.effective_policy, binding.effective_policy_hash
             FROM agent_secret_bindings AS binding
             JOIN secret_imports AS imported ON imported.id = binding.import_id
             JOIN secret_grants AS source_grant
               ON source_grant.id = imported.grant_id
             JOIN secrets AS secret ON secret.id = imported.secret_id
             WHERE binding.instance_revision_id = $1
               AND binding.status = 'active'
               AND imported.status = 'active'
               AND source_grant.status = 'active'
               AND secret.status = 'active'
               AND (source_grant.expires_at IS NULL
                    OR source_grant.expires_at > now())
             ORDER BY binding.slot_key",
        )
        .bind(command.expected_revision_id.as_uuid())
        .fetch_all(&mut *tx)
        .await?;
        let expected: Vec<Uuid> = serde_json::from_value(current.secret_bindings)?;
        if carried.len() != expected.len() {
            return Err(ReleaseServiceError::SecretBindingUnavailable);
        }
        for binding in &carried {
            self.require(
                &mut tx,
                identity,
                if binding.delivery_mode == "raw" {
                    Permission::BindRaw
                } else {
                    Permission::BindBrokered
                },
                ObjectRef::new(ObjectType::SecretImport, binding.import_id),
            )
            .await?;
        }
        let new_binding_ids = carried.iter().map(|_| Uuid::new_v4()).collect::<Vec<_>>();
        let bound_slots = carried
            .iter()
            .map(|binding| binding.slot_key.as_str())
            .collect::<std::collections::HashSet<_>>();
        let diagnostics = required_slot_diagnostics(&current.secret_slot_schema, &bound_slots)?;
        let runnable = diagnostics.is_empty();
        insert_revision_with_binding_ids(
            &mut tx,
            &command,
            current.release_agent_id,
            &parameters,
            &effective,
            runnable,
            &diagnostics,
            &new_binding_ids,
            identity,
        )
        .await?;
        for (binding, binding_id) in carried.iter().zip(&new_binding_ids) {
            clone_revision_binding(
                &mut tx,
                *binding_id,
                command.new_revision_id,
                binding,
                identity,
            )
            .await?;
        }
        let activated = sqlx::query(
            "UPDATE agent_instances
             SET active_revision_id = $3, version = version + 1,
                 updated_at = now()
             WHERE id = $1 AND active_revision_id = $2
               AND state IN ('active', 'update_rejected')",
        )
        .bind(command.instance_id.as_uuid())
        .bind(command.expected_revision_id.as_uuid())
        .bind(command.new_revision_id.as_uuid())
        .execute(&mut *tx)
        .await?;
        if activated.rows_affected() != 1 {
            return Err(ReleaseServiceError::StaleInstanceRevision);
        }
        record_command(
            &mut tx,
            command.command_key,
            "revise_instance",
            command.instance_id.as_uuid(),
            Some(command.new_revision_id.as_uuid()),
            Some(identity),
        )
        .await?;
        append_instance_event(
            &mut tx,
            command.instance_id,
            Some(command.new_revision_id),
            "instance.revised",
            identity,
            json!({"runnable": runnable}),
        )
        .await?;
        append_event(
            &mut tx,
            command.instance_id.as_uuid(),
            "hephaestus.agent_instance.revised.v1",
            "agent_instance.revised.v1",
            json!({
                "schema_version": 1,
                "instance_id": command.instance_id,
                "revision_id": command.new_revision_id,
                "expected_revision_id": command.expected_revision_id,
                "runnable": runnable,
            }),
        )
        .await?;
        tx.commit().await?;
        Ok(command.new_revision_id)
    }

    /// Creates a release-update candidate. Invalid state capability, missing
    /// hook, parameter, secret, or policy candidates are persisted as rejected
    /// diagnostics without closing the run gate.
    ///
    /// # Errors
    ///
    /// Fails for denial, stale/concurrent update, family mismatch,
    /// idempotency conflict, invalid stored data, or database failure.
    #[allow(clippy::too_many_lines)]
    #[tracing::instrument(
        skip_all,
        fields(
            actor_id = %identity.user_id,
            request_id = %identity.request_id,
            update_id = %command.update_id,
            instance_id = %command.instance_id,
            candidate_revision_id = %command.candidate_revision_id
        )
    )]
    pub async fn create_update(
        &self,
        identity: &AuthenticatedIdentity,
        command: CreateInstanceUpdate,
    ) -> Result<AgentUpdateId, ReleaseServiceError> {
        let mut tx = begin_actor_transaction(&self.pool, identity).await?;
        self.require(
            &mut tx,
            identity,
            Permission::CanUpdate,
            ObjectRef::new(ObjectType::AgentInstance, command.instance_id.as_uuid()),
        )
        .await?;
        self.require(
            &mut tx,
            identity,
            Permission::CanUse,
            ObjectRef::new(
                ObjectType::ReleaseAgent,
                command.candidate_release_agent_id.as_uuid(),
            ),
        )
        .await?;
        if let Some((id, _)) =
            existing_command(&mut tx, command.command_key, "create_update").await?
        {
            tx.commit().await?;
            return Ok(AgentUpdateId::from_uuid(id));
        }
        let current: UpdateCurrentRow = sqlx::query_as(
            "SELECT instance.active_revision_id, instance.family_id,
                    instance.state, instance.run_gate_open,
                    current_agent.requires_state,
                    revision.secret_bindings
             FROM agent_instances AS instance
             JOIN agent_instance_revisions AS revision
               ON revision.id = instance.active_revision_id
             JOIN release_agents AS current_agent
               ON current_agent.id = revision.release_agent_id
             WHERE instance.id = $1
             FOR UPDATE OF instance",
        )
        .bind(command.instance_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(ReleaseServiceError::Unavailable)?;
        if current.active_revision_id != Some(command.expected_revision_id.as_uuid()) {
            return Err(ReleaseServiceError::StaleInstanceRevision);
        }
        if !["active", "update_rejected"].contains(&current.state.as_str())
            || !current.run_gate_open
        {
            return Err(ReleaseServiceError::ConcurrentUpdate);
        }
        let candidate: UpdateCandidateRow = sqlx::query_as(
            "SELECT candidate.family_id, candidate.parameter_schema,
                    candidate.secret_slot_schema, candidate.runtime_contract,
                    candidate.requires_state, candidate.update_hook
             FROM release_agents AS candidate
             JOIN releases AS release ON release.id = candidate.release_id
             WHERE candidate.id = $1 AND release.state = 'published'",
        )
        .bind(command.candidate_release_agent_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(ReleaseServiceError::Unavailable)?;
        if candidate.family_id != current.family_id {
            return Err(ReleaseServiceError::AgentFamilyMismatch);
        }
        let declarations: Vec<ParameterDeclaration> =
            serde_json::from_value(candidate.parameter_schema)?;
        let parameters = ParameterDocument::resolve(&declarations, &command.parameters)
            .map_err(ReleaseServiceError::InvalidParameters)?;
        let release_policy = policy_from_contract(&candidate.runtime_contract)?;
        let effective = RuntimePolicy::resolve(
            &release_policy,
            &command.selected_policy,
            &command.platform_policy,
        )?;
        let carried: Vec<RevisionBindingRow> = sqlx::query_as(
            "SELECT binding.import_id, binding.slot_key,
                    binding.delivery_mode, binding.phases,
                    binding.attachment_ids, binding.destinations,
                    binding.effective_policy, binding.effective_policy_hash
             FROM agent_secret_bindings AS binding
             JOIN secret_imports AS imported ON imported.id = binding.import_id
             JOIN secret_grants AS source_grant
               ON source_grant.id = imported.grant_id
             JOIN secrets AS secret ON secret.id = imported.secret_id
             WHERE binding.instance_revision_id = $1
               AND binding.status = 'active'
               AND imported.status = 'active'
               AND source_grant.status = 'active'
               AND secret.status = 'active'
               AND (source_grant.expires_at IS NULL
                    OR source_grant.expires_at > now())
             ORDER BY binding.slot_key",
        )
        .bind(command.expected_revision_id.as_uuid())
        .fetch_all(&mut *tx)
        .await?;
        let expected: Vec<Uuid> = serde_json::from_value(current.secret_bindings)?;
        let mut diagnostics = Vec::new();
        if carried.len() != expected.len() {
            diagnostics.push(json!({
                "code": "secret_binding_unavailable",
                "field": "secret_slots"
            }));
        }
        for binding in &carried {
            self.require(
                &mut tx,
                identity,
                if binding.delivery_mode == "raw" {
                    Permission::BindRaw
                } else {
                    Permission::BindBrokered
                },
                ObjectRef::new(ObjectType::SecretImport, binding.import_id),
            )
            .await?;
            if !candidate_accepts_binding(&candidate.secret_slot_schema, binding) {
                diagnostics.push(json!({
                    "code": "secret_binding_incompatible",
                    "field": format!("secret_slots.{}", binding.slot_key)
                }));
            }
        }
        let bound_slots = carried
            .iter()
            .map(|binding| binding.slot_key.as_str())
            .collect::<std::collections::HashSet<_>>();
        diagnostics.extend(required_slot_diagnostics(
            &candidate.secret_slot_schema,
            &bound_slots,
        )?);
        diagnostics.extend(update_contract_diagnostics(
            current.requires_state,
            candidate.requires_state,
            candidate.update_hook.as_ref(),
        ));
        let runnable = diagnostics.is_empty();
        let new_binding_ids = carried.iter().map(|_| Uuid::new_v4()).collect::<Vec<_>>();
        insert_update_candidate_revision(
            &mut tx,
            &command,
            &parameters,
            &effective,
            runnable,
            &diagnostics,
            &new_binding_ids,
            identity,
        )
        .await?;
        for (binding, binding_id) in carried.iter().zip(&new_binding_ids) {
            clone_revision_binding(
                &mut tx,
                *binding_id,
                command.candidate_revision_id,
                binding,
                identity,
            )
            .await?;
        }
        let update_state = if runnable { "draining" } else { "rejected" };
        sqlx::query(
            "INSERT INTO agent_updates
             (id, instance_id, expected_current_revision_id,
              candidate_revision_id, state, diagnostics, final_decision,
              actor_id, completed_at)
             VALUES ($1, $2, $3, $4, $5, $6,
                     CASE WHEN $5 = 'rejected' THEN 'agent_rejected' END,
                     $7, CASE WHEN $5 = 'rejected' THEN now() END)",
        )
        .bind(command.update_id.as_uuid())
        .bind(command.instance_id.as_uuid())
        .bind(command.expected_revision_id.as_uuid())
        .bind(command.candidate_revision_id.as_uuid())
        .bind(update_state)
        .bind(serde_json::to_value(&diagnostics)?)
        .bind(identity.user_id.as_uuid())
        .execute(&mut *tx)
        .await?;
        if runnable {
            let closed = sqlx::query(
                "UPDATE agent_instances
                 SET run_gate_open = false, state = 'update_draining',
                     version = version + 1, updated_at = now()
                 WHERE id = $1 AND active_revision_id = $2
                   AND run_gate_open AND state IN ('active', 'update_rejected')",
            )
            .bind(command.instance_id.as_uuid())
            .bind(command.expected_revision_id.as_uuid())
            .execute(&mut *tx)
            .await?;
            if closed.rows_affected() != 1 {
                return Err(ReleaseServiceError::ConcurrentUpdate);
            }
        }
        record_command(
            &mut tx,
            command.command_key,
            "create_update",
            command.update_id.as_uuid(),
            Some(command.candidate_revision_id.as_uuid()),
            Some(identity),
        )
        .await?;
        append_instance_event(
            &mut tx,
            command.instance_id,
            Some(command.candidate_revision_id),
            if runnable {
                "update.draining"
            } else {
                "update.rejected"
            },
            identity,
            json!({
                "update_id": command.update_id,
                "diagnostics": diagnostics,
            }),
        )
        .await?;
        append_event(
            &mut tx,
            command.update_id.as_uuid(),
            "hephaestus.agent_update.requested.v1",
            "agent_update.requested.v1",
            json!({
                "schema_version": 1,
                "update_id": command.update_id,
                "instance_id": command.instance_id,
                "expected_revision_id": command.expected_revision_id,
                "candidate_revision_id": command.candidate_revision_id,
                "state": update_state,
            }),
        )
        .await?;
        if !runnable {
            append_event(
                &mut tx,
                command.update_id.as_uuid(),
                "hephaestus.agent_update.rejected.v1",
                "agent_update.rejected.v1",
                json!({
                    "update_id": command.update_id,
                    "instance_id": command.instance_id,
                    "expected_revision_id": command.expected_revision_id,
                    "candidate_revision_id": command.candidate_revision_id,
                    "reason": "candidate_not_runnable",
                    "diagnostics": diagnostics,
                }),
            )
            .await?;
        }
        tx.commit().await?;
        Ok(command.update_id)
    }

    /// Enters the isolated hook only after all pre-gate normal work drains and
    /// creates the exact queued update run. The run orchestrator acquires the
    /// optional state volume under its normal fenced exclusive lease.
    ///
    /// # Errors
    ///
    /// Fails for denial, undrained work, stale lifecycle, idempotency conflict,
    /// or database failure.
    #[allow(clippy::too_many_lines)]
    #[tracing::instrument(
        skip_all,
        fields(
            actor_id = %identity.user_id,
            request_id = %identity.request_id,
            update_id = %command.update_id,
            run_id = %command.hook_run_id
        )
    )]
    pub async fn begin_update_hook(
        &self,
        identity: &AuthenticatedIdentity,
        command: BeginUpdateHook,
    ) -> Result<(), ReleaseServiceError> {
        let mut tx = begin_actor_transaction(&self.pool, identity).await?;
        let update: UpdateHookAdmissionRow = sqlx::query_as(
            "SELECT update.instance_id, update.candidate_revision_id, update.state,
                    instance.state AS instance_state,
                    update.created_at,
                    candidate.release_agent_id,
                    release_agent.release_id,
                    release_agent.requires_state
             FROM agent_updates AS update
             JOIN agent_instances AS instance ON instance.id = update.instance_id
             JOIN agent_instance_revisions AS candidate
               ON candidate.id = update.candidate_revision_id
              AND candidate.instance_id = update.instance_id
             JOIN release_agents AS release_agent
               ON release_agent.id = candidate.release_agent_id
             WHERE update.id = $1
             FOR UPDATE OF update, instance",
        )
        .bind(command.update_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(ReleaseServiceError::Unavailable)?;
        self.require(
            &mut tx,
            identity,
            Permission::CanUpdate,
            ObjectRef::new(ObjectType::AgentInstance, update.instance_id),
        )
        .await?;
        if existing_command(&mut tx, command.command_key, "begin_update_hook")
            .await?
            .is_some()
        {
            tx.commit().await?;
            return Ok(());
        }
        if update.state != "draining" || update.instance_state != "update_draining" {
            return Err(ReleaseServiceError::InvalidUpdateLifecycle);
        }
        let pending: bool = sqlx::query_scalar(
            "SELECT EXISTS(
                 SELECT 1 FROM run_requests
                 WHERE instance_id = $1
                   AND request_kind = 'instance_normal'
                   AND dispatch_state = 'pending'
                   AND created_at <= $2
             ) OR EXISTS(
                 SELECT 1
                 FROM run_instance_provenance AS provenance
                 JOIN runs ON runs.id = provenance.run_id
                 WHERE provenance.instance_id = $1
                   AND provenance.phase = 'normal'
                   AND runs.state NOT IN (
                       'succeeded', 'failed', 'cancelled', 'cleaning_up',
                       'cleaned_up'
                   )
             )",
        )
        .bind(update.instance_id)
        .bind(update.created_at)
        .fetch_one(&mut *tx)
        .await?;
        if pending {
            return Err(ReleaseServiceError::UpdateDrainPending);
        }
        let start_id = CommandId::new();
        sqlx::query(
            "INSERT INTO runs
             (id, instance_id, instance_revision_id, release_id,
              release_agent_id, run_kind, command_id, state, requires_state,
              created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, 'update', $6, 'queued', $7,
                     now(), now())",
        )
        .bind(command.hook_run_id.as_uuid())
        .bind(update.instance_id)
        .bind(update.candidate_revision_id)
        .bind(update.release_id)
        .bind(update.release_agent_id)
        .bind(start_id.as_uuid())
        .bind(update.requires_state)
        .execute(&mut *tx)
        .await?;
        let changed = sqlx::query(
            "UPDATE agent_updates
             SET state = 'hook_running', hook_run_id = $2, updated_at = now()
             WHERE id = $1 AND state = 'draining'",
        )
        .bind(command.update_id.as_uuid())
        .bind(command.hook_run_id.as_uuid())
        .execute(&mut *tx)
        .await?;
        if changed.rows_affected() != 1 {
            return Err(ReleaseServiceError::InvalidUpdateLifecycle);
        }
        sqlx::query(
            "UPDATE agent_instances
             SET state = 'updating', version = version + 1, updated_at = now()
             WHERE id = $1 AND state = 'update_draining'
               AND NOT run_gate_open",
        )
        .bind(update.instance_id)
        .execute(&mut *tx)
        .await?;
        record_command(
            &mut tx,
            command.command_key,
            "begin_update_hook",
            command.update_id.as_uuid(),
            Some(command.hook_run_id.as_uuid()),
            Some(identity),
        )
        .await?;
        append_event(
            &mut tx,
            command.update_id.as_uuid(),
            "hephaestus.agent_update.hook_started.v1",
            "agent_update.hook_started.v1",
            json!({
                "schema_version": 1,
                "update_id": command.update_id,
                "hook_run_id": command.hook_run_id,
            }),
        )
        .await?;
        let start = StartRun {
            command_id: start_id,
            run_id: command.hook_run_id,
            instance_id: AgentInstanceId::from_uuid(update.instance_id),
            instance_revision_id: AgentInstanceRevisionId::from_uuid(update.candidate_revision_id),
            release_id: ReleaseId::from_uuid(update.release_id),
            release_agent_id: ReleaseAgentId::from_uuid(update.release_agent_id),
            attachment_id: None,
            kind: RunKind::Update,
            requires_state: update.requires_state,
        };
        append_event(
            &mut tx,
            command.hook_run_id.as_uuid(),
            RUN_START_SUBJECT,
            "run.start.v1",
            serde_json::to_value(start)?,
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Applies an explicit operator decision to a paused update.
    ///
    /// Retrying is permitted only from the compatibility-unknown path and
    /// retains the stable update ID so an agent can deduplicate its own hook.
    /// Rejecting reopens the prior revision without claiming that Hephaestus
    /// rolled agent-owned state back. Resuming activation is permitted only
    /// after durable hook success.
    ///
    /// # Errors
    ///
    /// Fails for denial, an action/lifecycle mismatch, idempotency conflict,
    /// stale revision state, or database failure.
    #[allow(clippy::too_many_lines)]
    #[tracing::instrument(
        skip_all,
        fields(
            actor_id = %identity.user_id,
            request_id = %identity.request_id,
            update_id = %command.update_id,
            action = command.action.operation()
        )
    )]
    pub async fn recover_update(
        &self,
        identity: &AuthenticatedIdentity,
        command: RecoverInstanceUpdate,
    ) -> Result<UpdateRecoveryDecision, ReleaseServiceError> {
        let mut tx = begin_actor_transaction(&self.pool, identity).await?;
        let update: UpdateRecoveryRow = sqlx::query_as(
            "SELECT update.instance_id, update.expected_current_revision_id,
                    update.candidate_revision_id, update.state,
                    instance.active_revision_id,
                    instance.state AS instance_state,
                    instance.run_gate_open
             FROM agent_updates AS update
             JOIN agent_instances AS instance ON instance.id = update.instance_id
             WHERE update.id = $1
             FOR UPDATE OF update, instance",
        )
        .bind(command.update_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(ReleaseServiceError::Unavailable)?;
        self.require(
            &mut tx,
            identity,
            Permission::CanRecover,
            ObjectRef::new(ObjectType::AgentInstance, update.instance_id),
        )
        .await?;
        if existing_command(&mut tx, command.command_key, command.action.operation())
            .await?
            .is_some()
        {
            tx.commit().await?;
            return Ok(recovery_decision(command.action));
        }

        let event_type = match command.action {
            UpdateRecoveryAction::RetryHook => {
                if update.state != "compatibility_unknown"
                    || update.instance_state != "paused_unknown_state"
                    || update.run_gate_open
                    || update.active_revision_id != Some(update.expected_current_revision_id)
                {
                    return Err(ReleaseServiceError::InvalidUpdateLifecycle);
                }
                sqlx::query(
                    "UPDATE agent_updates
                     SET state = 'draining', hook_run_id = NULL,
                         hook_exit_code = NULL, hook_exit_signal = NULL,
                         final_decision = NULL, completed_at = NULL,
                         diagnostics = diagnostics || $2::jsonb,
                         updated_at = now()
                     WHERE id = $1",
                )
                .bind(command.update_id.as_uuid())
                .bind(json!([{
                    "code": "operator_retry_after_uncertain_hook",
                    "field": "update_hook"
                }]))
                .execute(&mut *tx)
                .await?;
                sqlx::query(
                    "UPDATE agent_instances
                     SET state = 'update_draining', version = version + 1,
                         updated_at = now()
                     WHERE id = $1",
                )
                .bind(update.instance_id)
                .execute(&mut *tx)
                .await?;
                mark_volume_lease(&mut tx, command.update_id, "released").await?;
                "update.recovery_retry_scheduled"
            }
            UpdateRecoveryAction::RejectCandidate => {
                if update.state != "compatibility_unknown"
                    || update.instance_state != "paused_unknown_state"
                    || update.run_gate_open
                    || update.active_revision_id != Some(update.expected_current_revision_id)
                {
                    return Err(ReleaseServiceError::InvalidUpdateLifecycle);
                }
                sqlx::query(
                    "UPDATE agent_updates
                     SET state = 'rejected', final_decision = 'recovery',
                         diagnostics = diagnostics || $2::jsonb,
                         completed_at = now(), updated_at = now()
                     WHERE id = $1",
                )
                .bind(command.update_id.as_uuid())
                .bind(json!([{
                    "code": "operator_rejected_uncertain_candidate",
                    "field": "agent_owned_state"
                }]))
                .execute(&mut *tx)
                .await?;
                reopen_after_update(
                    &mut tx,
                    command.update_id,
                    update.instance_id,
                    update.expected_current_revision_id,
                    "update_rejected",
                )
                .await?;
                "update.recovery_candidate_rejected"
            }
            UpdateRecoveryAction::ResumeActivation => {
                let active_revision = update
                    .active_revision_id
                    .ok_or(ReleaseServiceError::InvalidUpdateLifecycle)?;
                if update.state != "activation_recovery"
                    || update.instance_state != "paused_activation_recovery"
                    || update.run_gate_open
                    || ![
                        update.expected_current_revision_id,
                        update.candidate_revision_id,
                    ]
                    .contains(&active_revision)
                {
                    return Err(ReleaseServiceError::InvalidUpdateLifecycle);
                }
                sqlx::query(
                    "UPDATE agent_instances
                     SET active_revision_id = $2, state = 'active',
                         run_gate_open = true, version = version + 1,
                         updated_at = now()
                     WHERE id = $1",
                )
                .bind(update.instance_id)
                .bind(update.candidate_revision_id)
                .execute(&mut *tx)
                .await?;
                sqlx::query(
                    "UPDATE agent_updates
                     SET state = 'activated', final_decision = 'activated',
                         completed_at = now(), updated_at = now()
                     WHERE id = $1",
                )
                .bind(command.update_id.as_uuid())
                .execute(&mut *tx)
                .await?;
                mark_volume_lease(&mut tx, command.update_id, "released").await?;
                materialize_deferred_triggers(
                    &mut tx,
                    update.instance_id,
                    update.candidate_revision_id,
                )
                .await?;
                "update.recovery_activation_resumed"
            }
        };
        record_command(
            &mut tx,
            command.command_key,
            command.action.operation(),
            command.update_id.as_uuid(),
            Some(update.candidate_revision_id),
            Some(identity),
        )
        .await?;
        append_instance_event(
            &mut tx,
            AgentInstanceId::from_uuid(update.instance_id),
            Some(AgentInstanceRevisionId::from_uuid(
                update.candidate_revision_id,
            )),
            event_type,
            identity,
            json!({
                "update_id": command.update_id,
                "action": command.action.operation(),
                "host_rollback_claimed": false,
            }),
        )
        .await?;
        append_event(
            &mut tx,
            command.update_id.as_uuid(),
            "hephaestus.agent_update.recovered.v1",
            "agent_update.recovered.v1",
            json!({
                "schema_version": 1,
                "update_id": command.update_id,
                "instance_id": update.instance_id,
                "action": command.action.operation(),
            }),
        )
        .await?;
        tx.commit().await?;
        Ok(recovery_decision(command.action))
    }

    /// Records the hook terminal contract. Exit zero is the irreversible
    /// commit point but activation is a separate reconciliable transaction.
    ///
    /// # Errors
    ///
    /// Fails for stale lifecycle or database errors.
    #[tracing::instrument(skip_all, fields(%update_id))]
    pub async fn record_update_hook_result(
        &self,
        update_id: AgentUpdateId,
        result: UpdateHookResult,
    ) -> Result<UpdateDecision, ReleaseServiceError> {
        let mut tx = self.pool.begin().await?;
        let update: UpdateLifecycleRow = sqlx::query_as(
            "SELECT update.instance_id, update.expected_current_revision_id,
                    update.candidate_revision_id, update.state
             FROM agent_updates AS update
             JOIN agent_instances AS instance ON instance.id = update.instance_id
             WHERE update.id = $1
             FOR UPDATE OF update, instance",
        )
        .bind(update_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(ReleaseServiceError::Unavailable)?;
        if update.state != "hook_running" {
            return existing_terminal_decision(&update.state);
        }
        let decision = match result {
            UpdateHookResult::Committed => {
                sqlx::query(
                    "UPDATE agent_updates
                     SET state = 'hook_committed', hook_exit_code = 0,
                         updated_at = now()
                     WHERE id = $1",
                )
                .bind(update_id.as_uuid())
                .execute(&mut *tx)
                .await?;
                append_event(
                    &mut tx,
                    update_id.as_uuid(),
                    "hephaestus.agent_update.hook_committed.v1",
                    "agent_update.hook_committed.v1",
                    json!({"schema_version": 1, "update_id": update_id}),
                )
                .await?;
                UpdateDecision::ActivationRecovery
            }
            UpdateHookResult::Rejected(exit_code) => {
                if exit_code == 0 {
                    return Err(ReleaseServiceError::InvalidHookResult);
                }
                sqlx::query(
                    "UPDATE agent_updates
                     SET state = 'rejected', hook_exit_code = $2,
                         final_decision = 'agent_rejected',
                         completed_at = now(), updated_at = now()
                     WHERE id = $1",
                )
                .bind(update_id.as_uuid())
                .bind(exit_code)
                .execute(&mut *tx)
                .await?;
                reopen_after_update(
                    &mut tx,
                    update_id,
                    update.instance_id,
                    update.expected_current_revision_id,
                    "update_rejected",
                )
                .await?;
                append_rejected_update_event(&mut tx, update_id, &update, exit_code).await?;
                UpdateDecision::AgentRejected
            }
            UpdateHookResult::Uncertain => {
                sqlx::query(
                    "UPDATE agent_updates
                     SET state = 'compatibility_unknown',
                         final_decision = 'unknown',
                         completed_at = now(), updated_at = now()
                     WHERE id = $1",
                )
                .bind(update_id.as_uuid())
                .execute(&mut *tx)
                .await?;
                sqlx::query(
                    "UPDATE agent_instances
                     SET state = 'paused_unknown_state', run_gate_open = false,
                         version = version + 1, updated_at = now()
                     WHERE id = $1",
                )
                .bind(update.instance_id)
                .execute(&mut *tx)
                .await?;
                mark_volume_lease(&mut tx, update_id, "recovery_required").await?;
                append_uncertain_update_events(
                    &mut tx,
                    update_id,
                    &update,
                    "hook_result_uncertain",
                    "paused_unknown_state",
                )
                .await?;
                UpdateDecision::CompatibilityUnknown
            }
        };
        tx.commit().await?;
        Ok(decision)
    }

    /// Reconciles one cleaned update run into the durable update state machine.
    ///
    /// Successful hooks are activated immediately; explicit nonzero exits
    /// reopen the prior revision; every ambiguous terminal path pauses the
    /// instance for recovery. Repeated calls are idempotent.
    ///
    /// # Errors
    ///
    /// Returns a stable lifecycle error until the exact hook run is cleaned,
    /// or a database error when its result cannot be persisted.
    #[tracing::instrument(skip_all, fields(%run_id))]
    pub async fn reconcile_update_run(
        &self,
        run_id: RunId,
    ) -> Result<UpdateDecision, ReleaseServiceError> {
        let row: UpdateRunResultRow = sqlx::query_as(
            "SELECT update.id AS update_id, update.state AS update_state,
                    run.state AS run_state, run.outcome, run.exit_code,
                    run.exit_signal, run.failure
             FROM agent_updates AS update
             JOIN runs AS run ON run.id = update.hook_run_id
             WHERE update.hook_run_id = $1 AND run.run_kind = 'update'",
        )
        .bind(run_id.as_uuid())
        .fetch_optional(&self.pool)
        .await?
        .ok_or(ReleaseServiceError::Unavailable)?;
        let update_id = AgentUpdateId::from_uuid(row.update_id);
        match row.update_state.as_str() {
            "hook_committed" => return self.activate_committed_update(update_id).await,
            "hook_running" => {}
            state => return existing_terminal_decision(state),
        }
        if row.run_state != "cleaned_up" {
            return Err(ReleaseServiceError::InvalidUpdateLifecycle);
        }
        let result = match (
            row.outcome.as_deref(),
            row.exit_code,
            row.exit_signal,
            row.failure.as_deref(),
        ) {
            (Some("succeeded"), Some(0), None, _) => UpdateHookResult::Committed,
            (Some("failed"), Some(code), None, None) if code != 0 => {
                UpdateHookResult::Rejected(code)
            }
            (Some("failed" | "cancelled"), _, _, _) => UpdateHookResult::Uncertain,
            _ => return Err(ReleaseServiceError::InvalidHookResult),
        };
        let decision = self.record_update_hook_result(update_id, result).await?;
        if result == UpdateHookResult::Committed {
            self.activate_committed_update(update_id).await
        } else {
            Ok(decision)
        }
    }

    /// Activates an exact hook-committed candidate without re-running the hook
    /// or allowing later source revocation to veto the committed migration.
    ///
    /// # Errors
    ///
    /// Returns database failures; CAS anomalies are durably converted to
    /// activation-recovery state and returned as a decision.
    #[tracing::instrument(skip_all, fields(%update_id))]
    pub async fn activate_committed_update(
        &self,
        update_id: AgentUpdateId,
    ) -> Result<UpdateDecision, ReleaseServiceError> {
        let mut tx = self.pool.begin().await?;
        let update: UpdateLifecycleRow = sqlx::query_as(
            "SELECT update.instance_id, update.expected_current_revision_id,
                    update.candidate_revision_id, update.state
             FROM agent_updates AS update
             JOIN agent_instances AS instance ON instance.id = update.instance_id
             WHERE update.id = $1
             FOR UPDATE OF update, instance",
        )
        .bind(update_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(ReleaseServiceError::Unavailable)?;
        if update.state == "activated" {
            tx.commit().await?;
            return Ok(UpdateDecision::Activated);
        }
        if update.state != "hook_committed" {
            return Err(ReleaseServiceError::InvalidUpdateLifecycle);
        }
        let activated = sqlx::query(
            "UPDATE agent_instances
             SET active_revision_id = $3, state = 'active',
                 run_gate_open = true, version = version + 1,
                 updated_at = now()
             WHERE id = $1 AND active_revision_id = $2
               AND state = 'updating' AND NOT run_gate_open",
        )
        .bind(update.instance_id)
        .bind(update.expected_current_revision_id)
        .bind(update.candidate_revision_id)
        .execute(&mut *tx)
        .await?;
        if activated.rows_affected() != 1 {
            sqlx::query(
                "UPDATE agent_updates
                 SET state = 'activation_recovery',
                     final_decision = 'recovery', updated_at = now()
                 WHERE id = $1",
            )
            .bind(update_id.as_uuid())
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "UPDATE agent_instances
                 SET state = 'paused_activation_recovery',
                     run_gate_open = false, version = version + 1,
                     updated_at = now()
                 WHERE id = $1",
            )
            .bind(update.instance_id)
            .execute(&mut *tx)
            .await?;
            mark_volume_lease(&mut tx, update_id, "recovery_required").await?;
            append_uncertain_update_events(
                &mut tx,
                update_id,
                &update,
                "activation_compare_and_swap_failed",
                "paused_activation_recovery",
            )
            .await?;
            tx.commit().await?;
            return Ok(UpdateDecision::ActivationRecovery);
        }
        sqlx::query(
            "UPDATE agent_updates
             SET state = 'activated', final_decision = 'activated',
                 completed_at = now(), updated_at = now()
             WHERE id = $1",
        )
        .bind(update_id.as_uuid())
        .execute(&mut *tx)
        .await?;
        mark_volume_lease(&mut tx, update_id, "released").await?;
        materialize_deferred_triggers(&mut tx, update.instance_id, update.candidate_revision_id)
            .await?;
        append_event(
            &mut tx,
            update_id.as_uuid(),
            "hephaestus.agent_update.completed.v1",
            "agent_update.completed.v1",
            json!({
                "update_id": update_id,
                "instance_id": update.instance_id,
                "revision_id": update.candidate_revision_id,
                "decision": "activated",
            }),
        )
        .await?;
        tx.commit().await?;
        Ok(UpdateDecision::Activated)
    }

    async fn require(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        identity: &AuthenticatedIdentity,
        permission: Permission,
        object: ObjectRef,
    ) -> Result<(), ReleaseServiceError> {
        let decision = self
            .authorizer
            .check(tx, Subject::User(identity.user_id), permission, object)
            .await?;
        audit_decision(
            tx,
            identity.user_id,
            permission,
            object,
            decision,
            identity.request_id,
        )
        .await?;
        if decision == AuthorizationDecision::Allow {
            Ok(())
        } else {
            // The command transaction will roll back on denial. Persist the
            // denial independently so rejected privileged attempts remain
            // observable without committing any command-side state.
            let mut audit_tx = begin_actor_transaction(&self.pool, identity).await?;
            audit_decision(
                &mut audit_tx,
                identity.user_id,
                permission,
                object,
                decision,
                identity.request_id,
            )
            .await?;
            audit_tx.commit().await?;
            Err(ReleaseServiceError::AuthorizationDenied)
        }
    }
}

#[derive(sqlx::FromRow)]
struct BuildRow {
    repository_id: Uuid,
    source_commit: String,
    source_ref: String,
    build_definition_hash: Vec<u8>,
    state: String,
}

#[derive(sqlx::FromRow)]
struct ReleaseAgentRow {
    family_id: Uuid,
    parameter_schema: Value,
    secret_slot_schema: Value,
    runtime_contract: Value,
    requires_state: bool,
}

#[derive(sqlx::FromRow)]
struct RevisionUpdateRow {
    active_revision_id: Option<Uuid>,
    release_agent_id: Uuid,
    secret_bindings: Value,
    parameter_schema: Value,
    secret_slot_schema: Value,
    runtime_contract: Value,
}

#[derive(sqlx::FromRow)]
struct RevisionBindingRow {
    import_id: Uuid,
    slot_key: String,
    delivery_mode: String,
    phases: Vec<String>,
    attachment_ids: Vec<Uuid>,
    destinations: Vec<String>,
    effective_policy: Value,
    effective_policy_hash: Vec<u8>,
}

#[derive(sqlx::FromRow)]
struct UpdateCurrentRow {
    active_revision_id: Option<Uuid>,
    family_id: Uuid,
    state: String,
    run_gate_open: bool,
    requires_state: bool,
    secret_bindings: Value,
}

#[derive(sqlx::FromRow)]
struct UpdateRecoveryRow {
    instance_id: Uuid,
    expected_current_revision_id: Uuid,
    candidate_revision_id: Uuid,
    state: String,
    active_revision_id: Option<Uuid>,
    instance_state: String,
    run_gate_open: bool,
}

#[derive(sqlx::FromRow)]
struct UpdateCandidateRow {
    family_id: Uuid,
    parameter_schema: Value,
    secret_slot_schema: Value,
    runtime_contract: Value,
    requires_state: bool,
    update_hook: Option<Value>,
}

#[derive(sqlx::FromRow)]
struct UpdateLifecycleRow {
    instance_id: Uuid,
    expected_current_revision_id: Uuid,
    candidate_revision_id: Uuid,
    state: String,
}

async fn append_rejected_update_event(
    tx: &mut Transaction<'_, Postgres>,
    update_id: AgentUpdateId,
    update: &UpdateLifecycleRow,
    exit_code: i32,
) -> Result<(), ReleaseServiceError> {
    append_event(
        tx,
        update_id.as_uuid(),
        "hephaestus.agent_update.rejected.v1",
        "agent_update.rejected.v1",
        json!({
            "update_id": update_id,
            "instance_id": update.instance_id,
            "expected_revision_id": update.expected_current_revision_id,
            "candidate_revision_id": update.candidate_revision_id,
            "reason": "hook_exit_nonzero",
            "exit_code": exit_code,
        }),
    )
    .await
}

async fn append_uncertain_update_events(
    tx: &mut Transaction<'_, Postgres>,
    update_id: AgentUpdateId,
    update: &UpdateLifecycleRow,
    reason: &str,
    paused_state: &str,
) -> Result<(), ReleaseServiceError> {
    append_event(
        tx,
        update_id.as_uuid(),
        "hephaestus.agent_update.uncertain.v1",
        "agent_update.uncertain.v1",
        json!({
            "update_id": update_id,
            "instance_id": update.instance_id,
            "expected_revision_id": update.expected_current_revision_id,
            "candidate_revision_id": update.candidate_revision_id,
            "reason": reason,
        }),
    )
    .await?;
    append_event(
        tx,
        update.instance_id,
        "hephaestus.agent_instance.paused.v1",
        "agent_instance.paused.v1",
        json!({
            "instance_id": update.instance_id,
            "update_id": update_id,
            "state": paused_state,
        }),
    )
    .await
}

#[derive(sqlx::FromRow)]
struct UpdateRunResultRow {
    update_id: Uuid,
    update_state: String,
    run_state: String,
    outcome: Option<String>,
    exit_code: Option<i32>,
    exit_signal: Option<i32>,
    failure: Option<String>,
}

#[derive(sqlx::FromRow)]
struct UpdateHookAdmissionRow {
    instance_id: Uuid,
    candidate_revision_id: Uuid,
    state: String,
    instance_state: String,
    created_at: OffsetDateTime,
    release_agent_id: Uuid,
    release_id: Uuid,
    requires_state: bool,
}

#[derive(sqlx::FromRow)]
struct DeferredMaterializationRow {
    id: Uuid,
    attachment_id: Uuid,
    repository_id: Uuid,
    target_ref: String,
    target_commit: String,
    source_id: Uuid,
    release_id: Uuid,
    release_agent_id: Uuid,
    platform_policy_version: String,
    requires_state: bool,
}

fn artifact_manifest_hash(artifacts: &[ReleaseArtifactInput]) -> ContentHash {
    let mut sorted = artifacts.iter().collect::<Vec<_>>();
    sorted.sort_unstable_by_key(|artifact| artifact.path.as_str());
    let mut digest = Sha256::new();
    for artifact in sorted {
        update_field(&mut digest, artifact.path.as_str().as_bytes());
        update_field(&mut digest, artifact.content_hash.as_bytes());
        update_field(&mut digest, &artifact.size_bytes.to_be_bytes());
        update_field(&mut digest, &artifact.mode.to_be_bytes());
    }
    ContentHash::digest(&digest.finalize())
}

fn update_field(digest: &mut Sha256, value: &[u8]) {
    digest.update(value.len().to_be_bytes());
    digest.update(value);
}

fn validate_artifact(artifact: &ReleaseArtifactInput) -> Result<(), ReleaseServiceError> {
    if artifact.media_type.is_empty() || artifact.media_type.len() > 256 || artifact.mode > 0o7777 {
        Err(ReleaseServiceError::InvalidArtifact)
    } else {
        Ok(())
    }
}

const fn artifact_kind_name(kind: ArtifactKind) -> &'static str {
    match kind {
        ArtifactKind::Executable => "executable",
        ArtifactKind::File => "file",
        ArtifactKind::Manifest => "manifest",
        ArtifactKind::BuildLog => "build_log",
    }
}

const fn runtime_policy(config: &AgentConfig) -> RuntimePolicy {
    RuntimePolicy {
        vcpus: config.resources.vcpus,
        memory_mib: config.resources.memory_mib,
        network: match config.network.profile {
            NetworkProfile::Disabled => NetworkAccess::Disabled,
            NetworkProfile::BrokerOnly => NetworkAccess::BrokerOnly,
            NetworkProfile::Egress => NetworkAccess::Egress,
        },
    }
}

fn parameter_schema(
    config: &AgentConfig,
) -> Result<Vec<ParameterDeclaration>, ReleaseServiceError> {
    config
        .parameters
        .iter()
        .map(|parameter| {
            let name = ParameterName::parse(parameter.name.clone())?;
            let value_type = match &parameter.value_type {
                agent_config::ParameterType::String {
                    minimum_length,
                    maximum_length,
                } => ParameterType::String {
                    minimum_length: *minimum_length,
                    maximum_length: *maximum_length,
                },
                agent_config::ParameterType::Integer { minimum, maximum } => {
                    ParameterType::Integer {
                        minimum: *minimum,
                        maximum: *maximum,
                    }
                }
                agent_config::ParameterType::Boolean => ParameterType::Boolean,
                agent_config::ParameterType::Enum { values } => ParameterType::Enum {
                    values: values.clone(),
                },
            };
            let default = parameter.default.as_ref().map(|value| match value {
                ParameterDefault::String(value) => ParameterValue::String(value.clone()),
                ParameterDefault::Integer(value) => ParameterValue::Integer(*value),
                ParameterDefault::Boolean(value) => ParameterValue::Boolean(*value),
            });
            Ok(ParameterDeclaration {
                name,
                value_type,
                required: parameter.required,
                default,
                sensitive: parameter.sensitive,
            })
        })
        .collect()
}

fn policy_from_contract(contract: &Value) -> Result<RuntimePolicy, ReleaseServiceError> {
    serde_json::from_value(
        contract
            .get("policy_ceiling")
            .cloned()
            .ok_or(ReleaseServiceError::InvalidStoredData)?,
    )
    .map_err(ReleaseServiceError::Serialization)
}

// Revision fields stay explicit to prevent accidental runtime-policy omission.
#[allow(clippy::too_many_arguments)]
async fn insert_revision(
    tx: &mut Transaction<'_, Postgres>,
    revision_id: AgentInstanceRevisionId,
    instance_id: AgentInstanceId,
    release_agent_id: ReleaseAgentId,
    parameters: &ParameterDocument,
    selection: &RuntimePolicy,
    effective: &RuntimePolicy,
    platform_policy_version: &str,
    runnable: bool,
    diagnostics: &[Value],
    identity: &AuthenticatedIdentity,
) -> Result<(), ReleaseServiceError> {
    let effective_bytes = serde_json::to_vec(effective)?;
    let effective_hash: [u8; 32] = Sha256::digest(&effective_bytes).into();
    sqlx::query(
        "INSERT INTO agent_instance_revisions
         (id, instance_id, release_agent_id, parameters, parameter_hash,
          secret_bindings, resource_selection, network_restriction,
          effective_runtime_policy, effective_policy_hash,
          platform_policy_version, runnable, diagnostics, created_by)
         VALUES ($1, $2, $3, $4, $5, '[]', $6, $7, $8, $9,
                 $10, $11, $12, $13)",
    )
    .bind(revision_id.as_uuid())
    .bind(instance_id.as_uuid())
    .bind(release_agent_id.as_uuid())
    .bind(serde_json::to_value(parameters.values())?)
    .bind(parameters.hash().as_bytes().as_slice())
    .bind(serde_json::to_value(selection)?)
    .bind(json!({"network": selection.network}))
    .bind(serde_json::to_value(effective)?)
    .bind(effective_hash.as_slice())
    .bind(platform_policy_version)
    .bind(runnable)
    .bind(serde_json::to_value(diagnostics)?)
    .bind(identity.user_id.as_uuid())
    .execute(&mut **tx)
    .await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn insert_revision_with_binding_ids(
    tx: &mut Transaction<'_, Postgres>,
    command: &ReviseInstance,
    release_agent_id: Uuid,
    parameters: &ParameterDocument,
    effective: &RuntimePolicy,
    runnable: bool,
    diagnostics: &[Value],
    binding_ids: &[Uuid],
    identity: &AuthenticatedIdentity,
) -> Result<(), ReleaseServiceError> {
    let effective_hash: [u8; 32] = Sha256::digest(serde_json::to_vec(effective)?).into();
    sqlx::query(
        "INSERT INTO agent_instance_revisions
         (id, instance_id, release_agent_id, parameters, parameter_hash,
          secret_bindings, resource_selection, network_restriction,
          effective_runtime_policy, effective_policy_hash,
          platform_policy_version, runnable, diagnostics, created_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10,
                 $11, $12, $13, $14)",
    )
    .bind(command.new_revision_id.as_uuid())
    .bind(command.instance_id.as_uuid())
    .bind(release_agent_id)
    .bind(serde_json::to_value(parameters.values())?)
    .bind(parameters.hash().as_bytes().as_slice())
    .bind(serde_json::to_value(binding_ids)?)
    .bind(serde_json::to_value(&command.selected_policy)?)
    .bind(json!({"network": command.selected_policy.network}))
    .bind(serde_json::to_value(effective)?)
    .bind(effective_hash.as_slice())
    .bind(&command.platform_policy_version)
    .bind(runnable)
    .bind(serde_json::to_value(diagnostics)?)
    .bind(identity.user_id.as_uuid())
    .execute(&mut **tx)
    .await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn insert_update_candidate_revision(
    tx: &mut Transaction<'_, Postgres>,
    command: &CreateInstanceUpdate,
    parameters: &ParameterDocument,
    effective: &RuntimePolicy,
    runnable: bool,
    diagnostics: &[Value],
    binding_ids: &[Uuid],
    identity: &AuthenticatedIdentity,
) -> Result<(), ReleaseServiceError> {
    let effective_hash: [u8; 32] = Sha256::digest(serde_json::to_vec(effective)?).into();
    sqlx::query(
        "INSERT INTO agent_instance_revisions
         (id, instance_id, release_agent_id, parameters, parameter_hash,
          secret_bindings, resource_selection, network_restriction,
          effective_runtime_policy, effective_policy_hash,
          platform_policy_version, runnable, diagnostics, created_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10,
                 $11, $12, $13, $14)",
    )
    .bind(command.candidate_revision_id.as_uuid())
    .bind(command.instance_id.as_uuid())
    .bind(command.candidate_release_agent_id.as_uuid())
    .bind(serde_json::to_value(parameters.values())?)
    .bind(parameters.hash().as_bytes().as_slice())
    .bind(serde_json::to_value(binding_ids)?)
    .bind(serde_json::to_value(&command.selected_policy)?)
    .bind(json!({"network": command.selected_policy.network}))
    .bind(serde_json::to_value(effective)?)
    .bind(effective_hash.as_slice())
    .bind(&command.platform_policy_version)
    .bind(runnable)
    .bind(serde_json::to_value(diagnostics)?)
    .bind(identity.user_id.as_uuid())
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn clone_revision_binding(
    tx: &mut Transaction<'_, Postgres>,
    binding_id: Uuid,
    revision_id: AgentInstanceRevisionId,
    binding: &RevisionBindingRow,
    identity: &AuthenticatedIdentity,
) -> Result<(), ReleaseServiceError> {
    sqlx::query(
        "INSERT INTO agent_secret_bindings
         (id, instance_revision_id, import_id, slot_key, delivery_mode,
          phases, attachment_ids, destinations, effective_policy,
          effective_policy_hash, status, created_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10,
                 'active', $11)",
    )
    .bind(binding_id)
    .bind(revision_id.as_uuid())
    .bind(binding.import_id)
    .bind(&binding.slot_key)
    .bind(&binding.delivery_mode)
    .bind(&binding.phases)
    .bind(&binding.attachment_ids)
    .bind(&binding.destinations)
    .bind(&binding.effective_policy)
    .bind(&binding.effective_policy_hash)
    .bind(identity.user_id.as_uuid())
    .execute(&mut **tx)
    .await?;
    Ok(())
}

fn required_slot_diagnostics(
    schema: &Value,
    bound_slots: &std::collections::HashSet<&str>,
) -> Result<Vec<Value>, ReleaseServiceError> {
    Ok(schema
        .as_array()
        .ok_or(ReleaseServiceError::InvalidStoredData)?
        .iter()
        .filter(|slot| slot.get("required").and_then(Value::as_bool) == Some(true))
        .filter_map(|slot| slot.get("key").and_then(Value::as_str))
        .filter(|slot| !bound_slots.contains(slot))
        .map(|slot| {
            json!({
                "code": "required_secret_binding_missing",
                "field": format!("secret_slots.{slot}")
            })
        })
        .collect())
}

fn candidate_accepts_binding(schema: &Value, binding: &RevisionBindingRow) -> bool {
    schema.as_array().is_some_and(|slots| {
        slots.iter().any(|slot| {
            slot.get("key").and_then(Value::as_str) == Some(&binding.slot_key)
                && slot
                    .get("delivery_modes")
                    .and_then(Value::as_array)
                    .is_some_and(|values| {
                        values
                            .iter()
                            .any(|value| value.as_str() == Some(&binding.delivery_mode))
                    })
                && slot
                    .get("phases")
                    .and_then(Value::as_array)
                    .is_some_and(|values| {
                        binding
                            .phases
                            .iter()
                            .all(|phase| values.iter().any(|value| value.as_str() == Some(phase)))
                    })
        })
    })
}

fn update_contract_diagnostics(
    current_requires_state: bool,
    candidate_requires_state: bool,
    candidate_update_hook: Option<&Value>,
) -> Vec<Value> {
    let mut diagnostics = Vec::new();
    if current_requires_state != candidate_requires_state {
        diagnostics.push(json!({
            "code": "state_capability_change_unsupported",
            "field": "state_volume.enabled"
        }));
    }
    if current_requires_state && candidate_update_hook.is_none() {
        diagnostics.push(json!({
            "code": "stateful_update_hook_missing",
            "field": "update_hook"
        }));
    }
    diagnostics
}

fn existing_terminal_decision(state: &str) -> Result<UpdateDecision, ReleaseServiceError> {
    match state {
        "activated" => Ok(UpdateDecision::Activated),
        "rejected" => Ok(UpdateDecision::AgentRejected),
        "compatibility_unknown" => Ok(UpdateDecision::CompatibilityUnknown),
        "hook_committed" | "activation_recovery" => Ok(UpdateDecision::ActivationRecovery),
        _ => Err(ReleaseServiceError::InvalidUpdateLifecycle),
    }
}

const fn recovery_decision(action: UpdateRecoveryAction) -> UpdateRecoveryDecision {
    match action {
        UpdateRecoveryAction::RetryHook => UpdateRecoveryDecision::HookRetryScheduled,
        UpdateRecoveryAction::RejectCandidate => UpdateRecoveryDecision::CandidateRejected,
        UpdateRecoveryAction::ResumeActivation => UpdateRecoveryDecision::CandidateActivated,
    }
}

async fn mark_volume_lease(
    tx: &mut Transaction<'_, Postgres>,
    update_id: AgentUpdateId,
    state: &str,
) -> Result<(), ReleaseServiceError> {
    let volume_id: Option<Uuid> = sqlx::query_scalar(
        "UPDATE agent_instance_volume_leases
         SET state = $2,
             released_at = CASE WHEN $2 = 'released' THEN now() END
         WHERE update_id = $1
           AND (
               state = 'active'
               OR ($2 = 'released' AND state = 'recovery_required')
           )
         RETURNING volume_id",
    )
    .bind(update_id.as_uuid())
    .bind(state)
    .fetch_optional(&mut **tx)
    .await?;
    if let Some(volume_id) = volume_id {
        sqlx::query(
            "UPDATE agent_instance_state_volumes
             SET state = CASE
                    WHEN $2 = 'released' THEN 'ready'
                    ELSE 'recovering'
                 END,
                 updated_at = now()
             WHERE id = $1",
        )
        .bind(volume_id)
        .bind(state)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

async fn reopen_after_update(
    tx: &mut Transaction<'_, Postgres>,
    update_id: AgentUpdateId,
    instance_id: Uuid,
    revision_id: Uuid,
    state: &str,
) -> Result<(), ReleaseServiceError> {
    sqlx::query(
        "UPDATE agent_instances
         SET state = $3, run_gate_open = true, version = version + 1,
             updated_at = now()
         WHERE id = $1 AND active_revision_id = $2
           AND state IN ('updating', 'paused_unknown_state')
           AND NOT run_gate_open",
    )
    .bind(instance_id)
    .bind(revision_id)
    .bind(state)
    .execute(&mut **tx)
    .await?;
    mark_volume_lease(tx, update_id, "released").await?;
    materialize_deferred_triggers(tx, instance_id, revision_id).await
}

async fn materialize_deferred_triggers(
    tx: &mut Transaction<'_, Postgres>,
    instance_id: Uuid,
    revision_id: Uuid,
) -> Result<(), ReleaseServiceError> {
    let rows: Vec<DeferredMaterializationRow> = sqlx::query_as(
        "SELECT deferred.id, deferred.attachment_id,
                deferred.repository_id, deferred.target_ref,
                deferred.target_commit, deferred.source_id,
                release.id AS release_id, release_agent.id AS release_agent_id,
                revision.platform_policy_version, release_agent.requires_state
         FROM deferred_agent_triggers AS deferred
         JOIN agent_attachments AS attachment
           ON attachment.id = deferred.attachment_id
          AND attachment.instance_id = deferred.instance_id
         JOIN agent_instance_revisions AS revision
           ON revision.id = $2 AND revision.instance_id = deferred.instance_id
         JOIN release_agents AS release_agent
           ON release_agent.id = revision.release_agent_id
         JOIN releases AS release ON release.id = release_agent.release_id
         WHERE deferred.instance_id = $1 AND deferred.state = 'deferred'
           AND attachment.enabled AND attachment.removed_at IS NULL
           AND revision.runnable AND release.state = 'published'
         ORDER BY deferred.created_at, deferred.id",
    )
    .bind(instance_id)
    .bind(revision_id)
    .fetch_all(&mut **tx)
    .await?;
    for row in rows {
        let request_id = Uuid::new_v4();
        let run_id = Uuid::new_v4();
        let command_id = Uuid::new_v4();
        let stored_request: Uuid = sqlx::query_scalar(
            "INSERT INTO run_requests
             (id, repository_id, commit_sha, git_ref, receive_id,
              run_id, command_id, instance_id, instance_revision_id,
              release_id, release_agent_id, attachment_id, request_kind,
              platform_policy_version, requires_state)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10,
                     $11, $12, 'instance_normal', $13, $14)
             ON CONFLICT (
                 attachment_id, instance_revision_id, commit_sha, git_ref,
                 receive_id, attempt
             ) WHERE request_kind = 'instance_normal'
             DO UPDATE SET repository_id = EXCLUDED.repository_id
             RETURNING id",
        )
        .bind(request_id)
        .bind(row.repository_id)
        .bind(&row.target_commit)
        .bind(&row.target_ref)
        .bind(row.source_id)
        .bind(run_id)
        .bind(command_id)
        .bind(instance_id)
        .bind(revision_id)
        .bind(row.release_id)
        .bind(row.release_agent_id)
        .bind(row.attachment_id)
        .bind(&row.platform_policy_version)
        .bind(row.requires_state)
        .fetch_one(&mut **tx)
        .await?;
        sqlx::query(
            "UPDATE deferred_agent_triggers
             SET state = 'materialized', run_request_id = $2,
                 resolved_at = now()
             WHERE id = $1 AND state = 'deferred'",
        )
        .bind(row.id)
        .bind(stored_request)
        .execute(&mut **tx)
        .await?;
        append_event(
            tx,
            stored_request,
            "hephaestus.instance.run.requested.v1",
            "instance.run.requested.v1",
            json!({
                "schema_version": 1,
                "run_request_id": stored_request,
                "run_id": run_id,
                "command_id": command_id,
                "instance_id": instance_id,
                "instance_revision_id": revision_id,
                "release_id": row.release_id,
                "release_agent_id": row.release_agent_id,
                "attachment_id": row.attachment_id,
                "target_repository_id": row.repository_id,
                "target_ref": row.target_ref,
                "target_commit": row.target_commit,
                "requires_state": row.requires_state,
            }),
        )
        .await?;
        append_deferred_start(tx, &row, instance_id, revision_id, run_id, command_id).await?;
    }
    deny_unmaterializable_triggers(tx, instance_id).await
}

async fn append_deferred_start(
    tx: &mut Transaction<'_, Postgres>,
    row: &DeferredMaterializationRow,
    instance_id: Uuid,
    revision_id: Uuid,
    run_id: Uuid,
    command_id: Uuid,
) -> Result<(), ReleaseServiceError> {
    let start = StartRun {
        command_id: CommandId::from_uuid(command_id),
        run_id: RunId::from_uuid(run_id),
        instance_id: AgentInstanceId::from_uuid(instance_id),
        instance_revision_id: AgentInstanceRevisionId::from_uuid(revision_id),
        release_id: ReleaseId::from_uuid(row.release_id),
        release_agent_id: ReleaseAgentId::from_uuid(row.release_agent_id),
        attachment_id: Some(AgentAttachmentId::from_uuid(row.attachment_id)),
        kind: RunKind::Normal,
        requires_state: row.requires_state,
    };
    append_event(
        tx,
        run_id,
        RUN_START_SUBJECT,
        "run.start.v1",
        serde_json::to_value(start)?,
    )
    .await
}

async fn deny_unmaterializable_triggers(
    tx: &mut Transaction<'_, Postgres>,
    instance_id: Uuid,
) -> Result<(), ReleaseServiceError> {
    sqlx::query(
        "UPDATE deferred_agent_triggers AS deferred
         SET state = 'denied',
             diagnostics = '[{\"code\":\"trigger_no_longer_authorized\"}]',
             resolved_at = now()
         WHERE deferred.instance_id = $1 AND deferred.state = 'deferred'
           AND NOT EXISTS (
               SELECT 1 FROM agent_attachments AS attachment
               WHERE attachment.id = deferred.attachment_id
                 AND attachment.enabled AND attachment.removed_at IS NULL
           )",
    )
    .bind(instance_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

fn ref_selector_string(selector: &RefSelector) -> String {
    match selector {
        RefSelector::Exact(value) => value.to_string(),
        RefSelector::Prefix(value) => format!("{value}/*"),
    }
}

const fn trigger_policy_name(policy: TriggerPolicy) -> &'static str {
    match policy {
        TriggerPolicy::Push => "push",
        TriggerPolicy::Manual => "manual",
        TriggerPolicy::PushAndManual => "push_and_manual",
    }
}

fn decode_hash(value: &str) -> Result<[u8; 32], ReleaseServiceError> {
    if value.len() != 64 {
        return Err(ReleaseServiceError::InvalidStoredData);
    }
    let mut output = [0_u8; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        let pair = std::str::from_utf8(pair).map_err(|_| ReleaseServiceError::InvalidStoredData)?;
        output[index] =
            u8::from_str_radix(pair, 16).map_err(|_| ReleaseServiceError::InvalidStoredData)?;
    }
    Ok(output)
}

fn hex_hash(value: &[u8; 32]) -> String {
    use std::fmt::Write as _;

    let mut output = String::with_capacity(64);
    for byte in value {
        write!(&mut output, "{byte:02x}").expect("writing to String cannot fail");
    }
    output
}

async fn existing_command(
    tx: &mut Transaction<'_, Postgres>,
    key: ReleaseCommandKey,
    operation: &str,
) -> Result<Option<(Uuid, Option<Uuid>)>, ReleaseServiceError> {
    let row: Option<(String, Uuid, Option<Uuid>)> = sqlx::query_as(
        "SELECT operation, aggregate_id, secondary_id
         FROM release_command_inbox WHERE command_key = $1",
    )
    .bind(key.as_bytes().as_slice())
    .fetch_optional(&mut **tx)
    .await?;
    match row {
        Some((stored, aggregate, secondary)) if stored == operation => {
            Ok(Some((aggregate, secondary)))
        }
        Some(_) => Err(ReleaseServiceError::IdempotencyConflict),
        None => Ok(None),
    }
}

async fn record_command(
    tx: &mut Transaction<'_, Postgres>,
    key: ReleaseCommandKey,
    operation: &str,
    aggregate_id: Uuid,
    secondary_id: Option<Uuid>,
    identity: Option<&AuthenticatedIdentity>,
) -> Result<(), ReleaseServiceError> {
    sqlx::query(
        "INSERT INTO release_command_inbox
         (command_key, operation, aggregate_id, secondary_id,
          actor_id, request_id)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(key.as_bytes().as_slice())
    .bind(operation)
    .bind(aggregate_id)
    .bind(secondary_id)
    .bind(identity.map(|value| value.user_id.as_uuid()))
    .bind(identity.map(|value| value.request_id.as_uuid()))
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn append_instance_event(
    tx: &mut Transaction<'_, Postgres>,
    instance_id: AgentInstanceId,
    revision_id: Option<AgentInstanceRevisionId>,
    event_type: &str,
    identity: &AuthenticatedIdentity,
    payload: Value,
) -> Result<(), ReleaseServiceError> {
    sqlx::query(
        "INSERT INTO agent_instance_events
         (id, instance_id, revision_id, event_type, actor_id, request_id,
          payload)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(Uuid::new_v4())
    .bind(instance_id.as_uuid())
    .bind(revision_id.map(AgentInstanceRevisionId::as_uuid))
    .bind(event_type)
    .bind(identity.user_id.as_uuid())
    .bind(identity.request_id.as_uuid())
    .bind(payload)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn append_event(
    tx: &mut Transaction<'_, Postgres>,
    aggregate_id: Uuid,
    subject: &str,
    event_type: &str,
    mut payload: Value,
) -> Result<(), ReleaseServiceError> {
    let event_id = Uuid::new_v4();
    enrich_message_payload(&mut payload, event_id);
    sqlx::query(
        "INSERT INTO outbox
         (id, aggregate_type, aggregate_id, subject, event_type,
          payload, occurred_at)
         VALUES ($1, 'release', $2, $3, $4, $5, $6)",
    )
    .bind(event_id)
    .bind(aggregate_id)
    .bind(subject)
    .bind(event_type)
    .bind(payload)
    .bind(OffsetDateTime::now_utc())
    .execute(&mut **tx)
    .await?;
    Ok(())
}

fn enrich_message_payload(payload: &mut Value, event_id: Uuid) {
    let Some(object) = payload.as_object_mut() else {
        return;
    };
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

/// Stable non-sensitive release service failure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ReleaseServiceError {
    /// Exact authorization was denied.
    #[error("release or instance command is not authorized")]
    AuthorizationDenied,
    /// Referenced build/release/agent/instance is unavailable.
    #[error("release or instance authority is unavailable")]
    Unavailable,
    /// Build is not at its sealed import boundary.
    #[error("build is not ready for immutable artifact import")]
    BuildNotImporting,
    /// No valid reusable configuration exists at the exact build commit.
    #[error("reusable release configuration is missing")]
    ReusableConfigurationMissing,
    /// Artifact set is empty.
    #[error("release artifact manifest is incomplete")]
    IncompleteArtifacts,
    /// Artifact metadata violates bounds.
    #[error("release artifact metadata is invalid")]
    InvalidArtifact,
    /// Typed parameter diagnostics prevented a revision.
    #[error("agent instance parameters are invalid")]
    InvalidParameters(Vec<release_domain::ParameterDiagnostic>),
    /// Stored JSON/provenance violates a domain invariant.
    #[error("stored release data is invalid")]
    InvalidStoredData,
    /// Expected active revision lost its compare-and-swap race.
    #[error("agent instance revision changed concurrently")]
    StaleInstanceRevision,
    /// One carried secret binding is no longer live and bindable.
    #[error("agent instance secret binding is unavailable")]
    SecretBindingUnavailable,
    /// Another update already closed the instance run gate.
    #[error("agent instance already has an active update")]
    ConcurrentUpdate,
    /// Candidate export belongs to another source agent family.
    #[error("agent update candidate belongs to another family")]
    AgentFamilyMismatch,
    /// Update or instance is not at the requested durable boundary.
    #[error("agent update lifecycle does not permit this operation")]
    InvalidUpdateLifecycle,
    /// Pre-gate normal requests or runs have not drained.
    #[error("agent update is waiting for normal runs to drain")]
    UpdateDrainPending,
    /// Persistent state volume is not ready for an exclusive update lease.
    #[error("agent update state volume is unavailable")]
    UpdateVolumeUnavailable,
    /// Update lease host or expiry violates bounds.
    #[error("agent update volume lease is invalid")]
    InvalidUpdateLease,
    /// Hook exit result is malformed.
    #[error("agent update hook result is invalid")]
    InvalidHookResult,
    /// One command identity was reused for another operation.
    #[error("release command idempotency identity conflicts")]
    IdempotencyConflict,
    /// Release domain validation failure.
    #[error(transparent)]
    Domain(#[from] release_domain::ReleaseValueError),
    /// Authorization provider failure.
    #[error(transparent)]
    Authorization(#[from] authz_domain::AuthzError),
    /// JSON serialization failure.
    #[error("release configuration serialization failed")]
    Serialization(#[from] serde_json::Error),
    /// Database failure.
    #[error("release persistence failed")]
    Database(#[from] sqlx::Error),
}

#[cfg(test)]
mod tests {
    use super::update_contract_diagnostics;
    use serde_json::json;

    #[test]
    fn update_state_contract_diagnostics_are_complete_and_stably_ordered() {
        assert_eq!(
            update_contract_diagnostics(true, false, None),
            vec![
                json!({
                    "code": "state_capability_change_unsupported",
                    "field": "state_volume.enabled"
                }),
                json!({
                    "code": "stateful_update_hook_missing",
                    "field": "update_hook"
                }),
            ]
        );
        assert_eq!(
            update_contract_diagnostics(false, true, Some(&json!({"command": "bin/update"}))),
            vec![json!({
                "code": "state_capability_change_unsupported",
                "field": "state_volume.enabled"
            })]
        );
        assert!(
            update_contract_diagnostics(true, true, Some(&json!({"command": "bin/update"})))
                .is_empty()
        );
    }
}
