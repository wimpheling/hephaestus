//! Authorized build query and request application operations.

use agent_config::AgentConfig;
use authz_postgres::begin_actor_transaction;
use identity_domain::AuthenticatedIdentity;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{FromRow, PgPool};
use std::collections::BTreeMap;
use time::OffsetDateTime;
use uuid::Uuid;

const BUILD_REQUESTED_SUBJECT: &str = "hephaestus.build.requested.v1";

/// Transport-neutral build lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildState {
    Queued,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

/// One runtime metric retained with a build execution.
pub struct BuildMetric {
    pub name: String,
    pub value: f64,
    pub labels: BTreeMap<String, String>,
}

/// Authorized build representation returned to a transport adapter.
pub struct BuildView {
    pub id: Uuid,
    pub repository_id: Uuid,
    pub state: BuildState,
    pub exit_code: Option<i32>,
    pub failure_code: Option<String>,
    pub logs: Vec<String>,
    pub metrics: Vec<BuildMetric>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
    pub source_commit: String,
    pub source_ref: String,
    pub build_definition_hash: String,
    pub release_id: Option<Uuid>,
    pub release_state: Option<String>,
    pub release_version: Option<String>,
    pub artifact_count: u32,
    pub trigger: String,
    pub agent_key: Option<String>,
    pub builder_image_id: Option<Uuid>,
    pub builder_image_key: Option<String>,
    pub builder_image_reference: Option<String>,
    pub configuration_hash: Option<String>,
    pub parsed_declaration: Value,
    pub build_policy: Value,
    pub started_at: Option<OffsetDateTime>,
    pub completed_at: Option<OffsetDateTime>,
    pub duration_milliseconds: Option<i64>,
    pub timeline: Vec<BuildTimelineEntry>,
    pub declared_artifacts: Vec<DeclaredArtifactView>,
    pub produced_artifacts: Vec<ProducedArtifactView>,
    pub artifact_manifest: Value,
}

/// One durable lifecycle observation for a build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildTimelineEntry {
    pub from_state: Option<String>,
    pub to_state: String,
    pub reason: String,
    pub occurred_at: OffsetDateTime,
}

/// One output declared by the immutable build configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclaredArtifactView {
    pub path: String,
    pub kind: String,
    pub media_type: Option<String>,
}

/// One output imported from a completed build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProducedArtifactView {
    pub path: String,
    pub kind: String,
    pub mode: u32,
    pub sha256: String,
    pub size_bytes: u64,
    pub media_type: String,
}

#[derive(Clone, Copy)]
pub struct BuildPage {
    pub size: i64,
    pub after: Option<Uuid>,
}

pub struct BuildPageResult {
    pub builds: Vec<BuildView>,
    pub next: Option<Uuid>,
}

impl BuildState {
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled)
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

pub fn encode_cursor(value: Uuid) -> String {
    value.to_string()
}

pub fn decode_cursor(value: &str) -> Option<Uuid> {
    Uuid::parse_str(value).ok()
}

/// A resumable cursor for the latest authorized build projection.
pub fn build_cursor(build: &BuildView) -> String {
    format!(
        "v1:build:{}:{}:{}:{}",
        build.id,
        build.updated_at.unix_timestamp_nanos(),
        build.state.as_str(),
        build.logs.len()
    )
}

/// Validated build request supplied by a transport adapter.
pub struct RequestBuild {
    pub repository_id: Uuid,
    pub source_commit: String,
    pub build_definition_hash: [u8; 32],
    pub configuration_hash: [u8; 32],
}

/// Durable result of requesting a build.
pub struct RequestedBuild {
    pub id: Uuid,
    pub state: BuildState,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

/// Typed build application failure.
#[derive(Debug, thiserror::Error)]
pub enum BuildError {
    /// No authorized row matched the requested resource.
    #[error("build resource was not found")]
    NotFound,
    /// Stored data did not satisfy the application contract.
    #[error("stored build data is invalid")]
    InvalidStoredData,
    /// The requested hashes do not identify the stored source configuration.
    #[error("build request does not match a valid source configuration")]
    FailedPrecondition,
    /// Persistence failed while evaluating the authorized operation.
    #[error("build persistence failed")]
    Persistence(#[source] sqlx::Error),
    /// Stored configuration could not be decoded.
    #[error("stored build configuration is invalid")]
    Serialization(#[source] serde_json::Error),
}

/// Typed failures for build actions that have additional lifecycle semantics.
#[derive(Debug, thiserror::Error)]
pub enum BuildActionError {
    #[error(transparent)]
    Application(#[from] BuildError),
    /// The current lifecycle state does not permit retry.
    #[error("build retry is not allowed in the current lifecycle state")]
    RetryNotAllowed,
    /// Retrying requires an attempt-reset capability not granted to the app role.
    #[error("retry is unavailable until durable build-attempt reset is supported")]
    RetryUnavailable,
    /// Verification requires a successful immutable input.
    #[error("verification rebuild requires a successful build")]
    VerificationNotAllowed,
    /// Verification needs durable comparison storage that is not in this schema.
    #[error("verification rebuild is unavailable until manifest comparison is durable")]
    VerificationUnavailable,
}

/// Executes build operations under transaction-local RLS identity.
#[derive(Clone)]
pub struct BuildApplication {
    pool: PgPool,
}

impl BuildApplication {
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn get_build(
        &self,
        identity: &AuthenticatedIdentity,
        id: Uuid,
    ) -> Result<BuildView, BuildError> {
        let mut transaction = begin_actor_transaction(&self.pool, identity)
            .await
            .map_err(BuildError::Persistence)?;
        let row = sqlx::query_as::<_, BuildRow>(
            "SELECT request.id, request.repository_id, request.state, execution.exit_code,
                    execution.failure_code, execution.logs, execution.metrics,
                    request.created_at,
                    COALESCE(execution.updated_at, request.completed_at,
                             request.started_at, request.created_at) AS updated_at,
                    request.source_commit, request.source_ref,
                    encode(request.build_definition_hash, 'hex') AS build_definition_hash,
                    request.build_trigger, request.agent_key,
                    request.builder_image_id, request.builder_image_key,
                    request.builder_image_reference,
                    encode(request.configuration_hash, 'hex') AS configuration_hash,
                    request.build_declaration, request.build_policy,
                    request.declared_artifacts, execution.started_at,
                    COALESCE(execution.completed_at, request.completed_at) AS completed_at,
                    execution.artifact_manifest,
                    release.id AS release_id, release.state AS release_state,
                    release.version AS release_version,
                    (SELECT count(*) FROM release_artifacts artifact
                     WHERE artifact.release_id = release.id)::bigint AS artifact_count
                    ,COALESCE((SELECT jsonb_agg(
                        jsonb_build_object(
                            'from_state', transition.from_state,
                            'to_state', transition.to_state,
                            'reason', transition.reason,
                            'occurred_at', transition.occurred_at
                        ) ORDER BY transition.occurred_at, transition.id
                    ) FROM build_state_transitions transition
                    WHERE transition.build_request_id = request.id), '[]'::jsonb) AS timeline
                    ,COALESCE((SELECT jsonb_agg(
                        jsonb_build_object(
                            'path', artifact.path,
                            'kind', artifact.kind,
                            'mode', artifact.mode,
                            'sha256', encode(artifact.content_hash, 'hex'),
                            'size_bytes', artifact.size_bytes,
                            'media_type', artifact.media_type
                        ) ORDER BY artifact.path, artifact.id
                    ) FROM release_artifacts artifact
                    WHERE artifact.release_id = release.id), '[]'::jsonb) AS produced_artifacts
             FROM build_requests request
             LEFT JOIN build_executions execution
               ON execution.build_request_id = request.id
             LEFT JOIN releases release ON release.build_request_id = request.id
             WHERE request.id = $1",
        )
        .bind(id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(BuildError::Persistence)?
        .ok_or(BuildError::NotFound)?;
        transaction
            .commit()
            .await
            .map_err(BuildError::Persistence)?;
        row.try_into()
    }

    pub async fn list_builds(
        &self,
        identity: &AuthenticatedIdentity,
        repository_id: Uuid,
        page: BuildPage,
    ) -> Result<BuildPageResult, BuildError> {
        let mut transaction = begin_actor_transaction(&self.pool, identity)
            .await
            .map_err(BuildError::Persistence)?;
        let rows = sqlx::query_as::<_, BuildRow>(
            "SELECT request.id, request.repository_id, request.state, execution.exit_code,
                    execution.failure_code, execution.logs, execution.metrics,
                    request.created_at,
                    COALESCE(execution.updated_at, request.completed_at,
                             request.started_at, request.created_at) AS updated_at,
                    request.source_commit, request.source_ref,
                    encode(request.build_definition_hash, 'hex') AS build_definition_hash,
                    request.build_trigger, request.agent_key,
                    request.builder_image_id, request.builder_image_key,
                    request.builder_image_reference,
                    encode(request.configuration_hash, 'hex') AS configuration_hash,
                    request.build_declaration, request.build_policy,
                    request.declared_artifacts, execution.started_at,
                    COALESCE(execution.completed_at, request.completed_at) AS completed_at,
                    execution.artifact_manifest,
                    release.id AS release_id, release.state AS release_state,
                    release.version AS release_version,
                    (SELECT count(*) FROM release_artifacts artifact
                     WHERE artifact.release_id = release.id)::bigint AS artifact_count
                    ,COALESCE((SELECT jsonb_agg(
                        jsonb_build_object(
                            'from_state', transition.from_state,
                            'to_state', transition.to_state,
                            'reason', transition.reason,
                            'occurred_at', transition.occurred_at
                        ) ORDER BY transition.occurred_at, transition.id
                    ) FROM build_state_transitions transition
                    WHERE transition.build_request_id = request.id), '[]'::jsonb) AS timeline
                    ,COALESCE((SELECT jsonb_agg(
                        jsonb_build_object(
                            'path', artifact.path,
                            'kind', artifact.kind,
                            'mode', artifact.mode,
                            'sha256', encode(artifact.content_hash, 'hex'),
                            'size_bytes', artifact.size_bytes,
                            'media_type', artifact.media_type
                        ) ORDER BY artifact.path, artifact.id
                    ) FROM release_artifacts artifact
                    WHERE artifact.release_id = release.id), '[]'::jsonb) AS produced_artifacts
             FROM build_requests request
             LEFT JOIN build_executions execution
               ON execution.build_request_id = request.id
             LEFT JOIN releases release ON release.build_request_id = request.id
             WHERE request.repository_id = $1
               AND ($2::uuid IS NULL OR (request.created_at, request.id) <
                    (SELECT cursor.created_at, cursor.id
                     FROM build_requests cursor WHERE cursor.id = $2))
             ORDER BY request.created_at DESC, request.id DESC
             LIMIT $3",
        )
        .bind(repository_id)
        .bind(page.after)
        .bind(page.size + 1)
        .fetch_all(&mut *transaction)
        .await
        .map_err(BuildError::Persistence)?;
        transaction
            .commit()
            .await
            .map_err(BuildError::Persistence)?;
        let size = usize::try_from(page.size).map_err(|_| BuildError::InvalidStoredData)?;
        let has_more = rows.len() > size;
        let builds: Vec<BuildView> = rows
            .into_iter()
            .take(size)
            .map(TryInto::try_into)
            .collect::<Result<Vec<_>, BuildError>>()?;
        let next = has_more
            .then(|| builds.last().map(|build| build.id))
            .flatten();
        Ok(BuildPageResult { builds, next })
    }

    /// Revalidates the build before reporting whether retry is possible.
    ///
    /// The application role intentionally cannot reset `build_executions`,
    /// and the current schema has no attempt history. Returning a typed
    /// service error keeps callers from pretending that a retry was queued.
    pub async fn retry_build(
        &self,
        identity: &AuthenticatedIdentity,
        id: Uuid,
    ) -> Result<RequestedBuild, BuildActionError> {
        let build = self
            .get_build(identity, id)
            .await
            .map_err(BuildActionError::Application)?;
        if !matches!(build.state, BuildState::Failed | BuildState::Cancelled) {
            return Err(BuildActionError::RetryNotAllowed);
        }
        Err(BuildActionError::RetryUnavailable)
    }

    /// Authorizes a verification request before rejecting the unsupported
    /// operation without creating a second, indistinguishable build row.
    pub async fn rebuild_for_verification(
        &self,
        identity: &AuthenticatedIdentity,
        id: Uuid,
    ) -> Result<RequestedBuild, BuildActionError> {
        let build = self
            .get_build(identity, id)
            .await
            .map_err(BuildActionError::Application)?;
        if build.state != BuildState::Succeeded {
            return Err(BuildActionError::VerificationNotAllowed);
        }
        Err(BuildActionError::VerificationUnavailable)
    }

    // The transaction intentionally keeps validation, durable request creation,
    // source linkage, and its outbox record visibly atomic.
    #[allow(clippy::too_many_lines)]
    pub async fn request_build(
        &self,
        identity: &AuthenticatedIdentity,
        request: RequestBuild,
    ) -> Result<RequestedBuild, BuildError> {
        let mut transaction = begin_actor_transaction(&self.pool, identity)
            .await
            .map_err(BuildError::Persistence)?;
        let configuration_hash = encode_hash(&request.configuration_hash);
        let source = sqlx::query_as::<_, BuildSourceRow>(
            "SELECT revision.receive_id, revision.config, reference.git_ref
             FROM agent_config_revisions revision
             JOIN git_refs reference
               ON reference.repository_id = revision.repository_id
              AND reference.commit_sha = revision.commit_sha
             WHERE revision.repository_id = $1
               AND revision.commit_sha = $2
               AND revision.normalized_config_hash = $3
               AND revision.status = 'valid'
               AND revision.config IS NOT NULL
             ORDER BY reference.git_ref, revision.created_at DESC, revision.id
             LIMIT 1",
        )
        .bind(request.repository_id)
        .bind(&request.source_commit)
        .bind(&configuration_hash)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(BuildError::Persistence)?
        .ok_or(BuildError::FailedPrecondition)?;
        let config: AgentConfig =
            serde_json::from_value(source.config).map_err(BuildError::Serialization)?;
        let build_bytes = serde_json::to_vec(&config.build).map_err(BuildError::Serialization)?;
        let actual_build_hash: [u8; 32] = Sha256::digest(build_bytes).into();
        if actual_build_hash != request.build_definition_hash {
            return Err(BuildError::FailedPrecondition);
        }
        let build = config
            .build
            .as_ref()
            .ok_or(BuildError::FailedPrecondition)?;
        let build_declaration = serde_json::to_value(build).map_err(BuildError::Serialization)?;
        let build_policy = json!({
            "resources": build.resources,
            "network": build.network,
        });
        let declared_artifacts =
            serde_json::to_value(&build.artifacts).map_err(BuildError::Serialization)?;
        let agent_key = config.agent.key.as_deref();

        let requested_id = Uuid::new_v4();
        let now = OffsetDateTime::now_utc();
        let row: (Uuid, OffsetDateTime, String, OffsetDateTime) = sqlx::query_as(
            "INSERT INTO build_requests
             (id, repository_id, source_commit, source_ref, origin_receive_id,
              build_definition_hash, state, created_by, created_at, build_trigger,
              agent_key, builder_image_id, builder_image_key, builder_image_reference,
              configuration_hash, build_declaration, build_policy, declared_artifacts)
             VALUES ($1, $2, $3, $4, $5, $6, 'queued', $7, $8, 'manual', $9,
                     (SELECT id FROM builder_images WHERE image_reference = $10),
                     (SELECT key FROM builder_images WHERE image_reference = $10),
                     $10, decode($11, 'hex'), $12, $13, $14)
             ON CONFLICT
               (repository_id, source_commit, source_ref, build_definition_hash)
             DO UPDATE SET repository_id = EXCLUDED.repository_id
             RETURNING id, created_at, state,
                       COALESCE(completed_at, started_at, created_at)",
        )
        .bind(requested_id)
        .bind(request.repository_id)
        .bind(&request.source_commit)
        .bind(&source.git_ref)
        .bind(source.receive_id)
        .bind(request.build_definition_hash.as_slice())
        .bind(identity.user_id.as_uuid())
        .bind(now)
        .bind(agent_key)
        .bind(&build.root_image)
        .bind(&configuration_hash)
        .bind(build_declaration)
        .bind(build_policy)
        .bind(declared_artifacts)
        .fetch_one(&mut *transaction)
        .await
        .map_err(BuildError::Persistence)?;
        sqlx::query(
            "INSERT INTO build_request_sources
             (build_request_id, receive_id, source_ref, source_commit, created_at)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT DO NOTHING",
        )
        .bind(row.0)
        .bind(source.receive_id)
        .bind(&source.git_ref)
        .bind(&request.source_commit)
        .bind(now)
        .execute(&mut *transaction)
        .await
        .map_err(BuildError::Persistence)?;
        if row.0 == requested_id {
            let event_id = Uuid::new_v4();
            sqlx::query(
                "INSERT INTO outbox
                 (id, aggregate_type, aggregate_id, subject, event_type, payload,
                  occurred_at)
                 VALUES ($1, 'forge', $2, $3, 'build.requested.v1', $4, $5)",
            )
            .bind(event_id)
            .bind(row.0)
            .bind(BUILD_REQUESTED_SUBJECT)
            .bind(json!({
                "schema_version": 1,
                "message_id": event_id,
                "idempotency_key": event_id,
                "request_id": identity.request_id,
                "trace_id": Value::Null,
                "build_request_id": row.0,
                "repository_id": request.repository_id,
                "source_commit": request.source_commit,
                "source_ref": source.git_ref,
                "receive_id": source.receive_id,
                "normalized_configuration_hash": configuration_hash,
                "build_definition_hash": encode_hash(&request.build_definition_hash),
            }))
            .bind(now)
            .execute(&mut *transaction)
            .await
            .map_err(BuildError::Persistence)?;
        }
        transaction
            .commit()
            .await
            .map_err(BuildError::Persistence)?;
        Ok(RequestedBuild {
            id: row.0,
            state: parse_state(&row.2)?,
            created_at: row.1,
            updated_at: row.3,
        })
    }
}

#[derive(FromRow)]
struct BuildRow {
    id: Uuid,
    repository_id: Uuid,
    state: String,
    exit_code: Option<i32>,
    failure_code: Option<String>,
    logs: Option<Value>,
    metrics: Option<Value>,
    created_at: OffsetDateTime,
    updated_at: OffsetDateTime,
    source_commit: String,
    source_ref: String,
    build_definition_hash: String,
    build_trigger: String,
    agent_key: Option<String>,
    builder_image_id: Option<Uuid>,
    builder_image_key: Option<String>,
    builder_image_reference: Option<String>,
    configuration_hash: Option<String>,
    build_declaration: Option<Value>,
    build_policy: Option<Value>,
    declared_artifacts: Option<Value>,
    started_at: Option<OffsetDateTime>,
    completed_at: Option<OffsetDateTime>,
    artifact_manifest: Option<Value>,
    release_id: Option<Uuid>,
    release_state: Option<String>,
    release_version: Option<String>,
    artifact_count: i64,
    timeline: Option<Value>,
    produced_artifacts: Option<Value>,
}

#[derive(FromRow)]
struct BuildSourceRow {
    receive_id: Uuid,
    config: Value,
    git_ref: String,
}

#[derive(Deserialize)]
struct StoredLog {
    stream: String,
    text: String,
}

#[derive(Deserialize)]
struct StoredMetric {
    name: String,
    value: f64,
    #[serde(default)]
    labels: BTreeMap<String, String>,
}

#[derive(Deserialize)]
struct StoredTimelineEntry {
    from_state: Option<String>,
    to_state: String,
    reason: String,
    occurred_at: OffsetDateTime,
}

impl TryFrom<StoredTimelineEntry> for BuildTimelineEntry {
    type Error = BuildError;

    fn try_from(value: StoredTimelineEntry) -> Result<Self, Self::Error> {
        if value.to_state.is_empty() || value.reason.is_empty() {
            return Err(BuildError::InvalidStoredData);
        }
        Ok(Self {
            from_state: value.from_state,
            to_state: value.to_state,
            reason: value.reason,
            occurred_at: value.occurred_at,
        })
    }
}

#[derive(Deserialize)]
struct StoredDeclaredArtifact {
    path: String,
    kind: String,
    media_type: Option<String>,
}

impl From<StoredDeclaredArtifact> for DeclaredArtifactView {
    fn from(value: StoredDeclaredArtifact) -> Self {
        Self {
            path: value.path,
            kind: value.kind,
            media_type: value.media_type,
        }
    }
}

#[derive(Deserialize)]
struct StoredProducedArtifact {
    path: String,
    kind: String,
    mode: i32,
    sha256: String,
    size_bytes: i64,
    media_type: String,
}

impl TryFrom<StoredProducedArtifact> for ProducedArtifactView {
    type Error = BuildError;

    fn try_from(value: StoredProducedArtifact) -> Result<Self, Self::Error> {
        Ok(Self {
            path: value.path,
            kind: value.kind,
            mode: u32::try_from(value.mode).map_err(|_| BuildError::InvalidStoredData)?,
            sha256: value.sha256,
            size_bytes: u64::try_from(value.size_bytes)
                .map_err(|_| BuildError::InvalidStoredData)?,
            media_type: value.media_type,
        })
    }
}

impl TryFrom<BuildRow> for BuildView {
    type Error = BuildError;

    fn try_from(row: BuildRow) -> Result<Self, Self::Error> {
        let state = parse_state(&row.state)?;
        let logs = serde_json::from_value::<Vec<StoredLog>>(
            row.logs.unwrap_or_else(|| Value::Array(Vec::new())),
        )
        .map_err(BuildError::Serialization)?
        .into_iter()
        .map(|entry| format!("[{}] {}", entry.stream, entry.text))
        .collect();
        let metrics = serde_json::from_value::<Vec<StoredMetric>>(
            row.metrics.unwrap_or_else(|| Value::Array(Vec::new())),
        )
        .map_err(BuildError::Serialization)?
        .into_iter()
        .map(|metric| BuildMetric {
            name: metric.name,
            value: metric.value,
            labels: metric.labels,
        })
        .collect();
        let timeline = serde_json::from_value::<Vec<StoredTimelineEntry>>(
            row.timeline.unwrap_or_else(|| Value::Array(Vec::new())),
        )
        .map_err(BuildError::Serialization)?
        .into_iter()
        .map(BuildTimelineEntry::try_from)
        .collect::<Result<Vec<_>, _>>()?;
        let declared_artifacts = serde_json::from_value::<Vec<StoredDeclaredArtifact>>(
            row.declared_artifacts
                .unwrap_or_else(|| Value::Array(Vec::new())),
        )
        .map_err(BuildError::Serialization)?
        .into_iter()
        .map(DeclaredArtifactView::from)
        .collect();
        let produced_artifacts = serde_json::from_value::<Vec<StoredProducedArtifact>>(
            row.produced_artifacts
                .unwrap_or_else(|| Value::Array(Vec::new())),
        )
        .map_err(BuildError::Serialization)?
        .into_iter()
        .map(ProducedArtifactView::try_from)
        .collect::<Result<Vec<_>, _>>()?;
        let duration_milliseconds = row.started_at.map(|started| {
            let milliseconds = row
                .completed_at
                .unwrap_or_else(OffsetDateTime::now_utc)
                .unix_timestamp_nanos()
                .saturating_sub(started.unix_timestamp_nanos())
                / 1_000_000;
            i64::try_from(milliseconds.max(0)).unwrap_or(i64::MAX)
        });
        Ok(Self {
            id: row.id,
            repository_id: row.repository_id,
            state,
            exit_code: row.exit_code,
            failure_code: row.failure_code,
            logs,
            metrics,
            created_at: row.created_at,
            updated_at: row.updated_at,
            source_commit: row.source_commit,
            source_ref: row.source_ref,
            build_definition_hash: row.build_definition_hash,
            release_id: row.release_id,
            release_state: row.release_state,
            release_version: row.release_version,
            artifact_count: u32::try_from(row.artifact_count)
                .map_err(|_| BuildError::InvalidStoredData)?,
            trigger: row.build_trigger,
            agent_key: row.agent_key,
            builder_image_id: row.builder_image_id,
            builder_image_key: row.builder_image_key,
            builder_image_reference: row.builder_image_reference,
            configuration_hash: row.configuration_hash,
            parsed_declaration: row
                .build_declaration
                .unwrap_or_else(|| Value::Object(serde_json::Map::new())),
            build_policy: row
                .build_policy
                .unwrap_or_else(|| Value::Object(serde_json::Map::new())),
            started_at: row.started_at,
            completed_at: row.completed_at,
            duration_milliseconds,
            timeline,
            declared_artifacts,
            produced_artifacts,
            artifact_manifest: row
                .artifact_manifest
                .unwrap_or_else(|| Value::Array(Vec::new())),
        })
    }
}

fn parse_state(value: &str) -> Result<BuildState, BuildError> {
    match value {
        "queued" => Ok(BuildState::Queued),
        "running" | "importing" => Ok(BuildState::Running),
        "succeeded" => Ok(BuildState::Succeeded),
        "failed" => Ok(BuildState::Failed),
        "cancelled" => Ok(BuildState::Cancelled),
        _ => Err(BuildError::InvalidStoredData),
    }
}

/// Decodes one canonical lowercase SHA-256 digest.
pub fn decode_hash(value: &str) -> Option<[u8; 32]> {
    if value.len() != 64 || value.bytes().any(|byte| !byte.is_ascii_hexdigit()) {
        return None;
    }
    let mut output = [0_u8; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        let high = hex_nibble(pair[0])?;
        let low = hex_nibble(pair[1])?;
        output[index] = high << 4 | low;
    }
    (encode_hash(&output) == value).then_some(output)
}

const fn hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        _ => None,
    }
}

fn encode_hash(value: &[u8; 32]) -> String {
    use std::fmt::Write as _;

    let mut output = String::with_capacity(64);
    for byte in value {
        write!(&mut output, "{byte:02x}").expect("writing to a String cannot fail");
    }
    output
}

#[cfg(test)]
mod tests {
    use super::{
        BuildActionError, BuildState, BuildView, build_cursor, decode_cursor, decode_hash,
        encode_cursor,
    };
    use time::OffsetDateTime;
    use uuid::Uuid;

    #[test]
    fn hash_decoder_requires_canonical_sha256_hex() {
        assert_eq!(decode_hash(&"ab".repeat(32)), Some([0xab; 32]));
        assert!(decode_hash(&"AB".repeat(32)).is_none());
        assert!(decode_hash("ab").is_none());
        assert!(decode_hash(&"gg".repeat(32)).is_none());
    }

    #[test]
    fn build_page_cursor_round_trips_opaque_uuid() {
        let id = Uuid::new_v4();
        assert_eq!(decode_cursor(&encode_cursor(id)), Some(id));
        assert!(decode_cursor("not-a-uuid").is_none());
    }

    #[test]
    fn build_watch_cursor_contains_the_authoritative_projection_version() {
        let id = Uuid::new_v4();
        let view = BuildView {
            id,
            repository_id: Uuid::new_v4(),
            state: BuildState::Running,
            exit_code: None,
            failure_code: None,
            logs: vec![String::from("[stdout] compiling")],
            metrics: Vec::new(),
            created_at: OffsetDateTime::UNIX_EPOCH,
            updated_at: OffsetDateTime::UNIX_EPOCH,
            source_commit: String::from("a"),
            source_ref: String::from("refs/heads/main"),
            build_definition_hash: String::from("b"),
            release_id: None,
            release_state: None,
            release_version: None,
            artifact_count: 0,
            trigger: String::from("manual"),
            agent_key: None,
            builder_image_id: None,
            builder_image_key: None,
            builder_image_reference: None,
            configuration_hash: None,
            parsed_declaration: serde_json::json!({}),
            build_policy: serde_json::json!({}),
            started_at: None,
            completed_at: None,
            duration_milliseconds: None,
            timeline: Vec::new(),
            declared_artifacts: Vec::new(),
            produced_artifacts: Vec::new(),
            artifact_manifest: serde_json::json!([]),
        };
        assert_eq!(build_cursor(&view), format!("v1:build:{id}:0:running:1"));
        assert!(!BuildState::Running.is_terminal());
        assert!(BuildState::Failed.is_terminal());
    }

    #[test]
    fn unsupported_actions_keep_precise_durable_reasons() {
        assert!(
            BuildActionError::RetryUnavailable
                .to_string()
                .contains("build-attempt")
        );
        assert!(
            BuildActionError::VerificationUnavailable
                .to_string()
                .contains("manifest comparison")
        );
    }
}
