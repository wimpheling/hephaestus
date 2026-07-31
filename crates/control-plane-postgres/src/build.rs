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
    pub state: BuildState,
    pub exit_code: Option<i32>,
    pub failure_code: Option<String>,
    pub logs: Vec<String>,
    pub metrics: Vec<BuildMetric>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
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

/// Executes build operations under transaction-local RLS identity.
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
            "SELECT request.id, request.state, execution.exit_code,
                    execution.failure_code, execution.logs, execution.metrics,
                    request.created_at,
                    COALESCE(execution.updated_at, request.completed_at,
                             request.started_at, request.created_at) AS updated_at
             FROM build_requests request
             LEFT JOIN build_executions execution
               ON execution.build_request_id = request.id
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

        let requested_id = Uuid::new_v4();
        let now = OffsetDateTime::now_utc();
        let row: (Uuid, OffsetDateTime, String, OffsetDateTime) = sqlx::query_as(
            "INSERT INTO build_requests
             (id, repository_id, source_commit, source_ref, origin_receive_id,
              build_definition_hash, state, created_by, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, 'queued', $7, $8)
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
    state: String,
    exit_code: Option<i32>,
    failure_code: Option<String>,
    logs: Option<Value>,
    metrics: Option<Value>,
    created_at: OffsetDateTime,
    updated_at: OffsetDateTime,
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
        Ok(Self {
            id: row.id,
            state,
            exit_code: row.exit_code,
            failure_code: row.failure_code,
            logs,
            metrics,
            created_at: row.created_at,
            updated_at: row.updated_at,
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
    use super::decode_hash;

    #[test]
    fn hash_decoder_requires_canonical_sha256_hex() {
        assert_eq!(decode_hash(&"ab".repeat(32)), Some([0xab; 32]));
        assert!(decode_hash(&"AB".repeat(32)).is_none());
        assert!(decode_hash("ab").is_none());
        assert!(decode_hash(&"gg".repeat(32)).is_none());
    }
}
