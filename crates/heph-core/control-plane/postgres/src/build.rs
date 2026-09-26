//! Authorized build query and request application operations.

use serde_json::Value;
use sqlx::PgPool;
use std::collections::BTreeMap;
use time::OffsetDateTime;
use uuid::Uuid;

const BUILD_REQUESTED_SUBJECT: &str = "hephaestus.build.requested.v1";
const BUILD_RETRY_REQUESTED_SUBJECT: &str = "hephaestus.build.retry.requested.v1";
const BUILD_VERIFY_REQUESTED_SUBJECT: &str = "hephaestus.build.verify.requested.v1";

mod actions;
mod models;
mod queries;
mod request;
mod ui_manifest;

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
    pub image_id: Option<Uuid>,
    pub image_key: Option<String>,
    pub image_reference: Option<String>,
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
    pub verifications: Vec<BuildVerificationView>,
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

/// One immutable-input verification execution retained for a build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildVerificationView {
    pub state: String,
    pub expected_manifest: Value,
    pub actual_manifest: Option<Value>,
    pub failure_code: Option<String>,
    pub created_at: OffsetDateTime,
    pub completed_at: Option<OffsetDateTime>,
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
    #[error("stored build configuration is invalid: {0}")]
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
        BuildActionError, BuildState, BuildVerificationView, BuildView, build_cursor,
        decode_cursor, decode_hash, encode_cursor,
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
            image_id: None,
            image_key: None,
            image_reference: None,
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
            verifications: vec![BuildVerificationView {
                state: String::from("failed"),
                expected_manifest: serde_json::json!([{"path": "expected"}]),
                actual_manifest: Some(serde_json::json!([{"path": "actual"}])),
                failure_code: Some(String::from("manifest_mismatch")),
                created_at: OffsetDateTime::UNIX_EPOCH,
                completed_at: Some(OffsetDateTime::UNIX_EPOCH),
            }],
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
