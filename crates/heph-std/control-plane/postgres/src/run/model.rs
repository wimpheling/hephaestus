//! Public run DTOs and the application handle.

use serde_json::Value;
use sqlx::{FromRow, PgPool};
use std::{collections::BTreeMap, path::PathBuf};
use time::OffsetDateTime;
use uuid::Uuid;

pub(super) const MAX_RESULT_PREVIEW_BYTES: u64 = 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum RunError {
    #[error("run was not found")]
    NotFound,
    #[error("run persistence failed")]
    Persistence(#[source] sqlx::Error),
    #[error("run page is invalid")]
    InvalidPage,
    #[error("control request conflicts with an earlier retry")]
    IdempotencyConflict,
    #[error("run result preview is unavailable")]
    PreviewUnavailable,
}

#[derive(Clone, Copy)]
pub struct Page {
    pub size: i64,
    pub after: Option<Uuid>,
}
pub struct PageResult<T> {
    pub values: Vec<T>,
    pub next: Option<String>,
}

#[derive(FromRow)]
pub struct AuthorizationProvenance {
    pub id: Uuid,
    pub authorization_model_version: String,
    pub normalized_hash: String,
}

#[derive(FromRow)]
pub struct HttpsUse {
    pub id: Uuid,
    pub request_id: Uuid,
    pub lease_id: Uuid,
    pub binding_id: Uuid,
    pub secret_version_id: Uuid,
    pub rule_id: Uuid,
    pub event_kind: String,
    pub decision: Option<String>,
    pub outcome: Option<String>,
    pub occurred_at: OffsetDateTime,
}

pub struct RunProvenance {
    pub snapshot: Option<AuthorizationProvenance>,
    pub uses: PageResult<HttpsUse>,
}

#[derive(FromRow)]
pub struct RunSummary {
    pub id: Uuid,
    pub state: String,
    pub outcome: Option<String>,
    pub run_kind: String,
    pub updated_at: OffsetDateTime,
    pub instance_id: Uuid,
    pub instance_name: String,
    pub repository_id: Option<Uuid>,
    pub repository_name: Option<String>,
    pub commit_sha: Option<String>,
    pub git_ref: Option<String>,
    pub release_id: Uuid,
    pub release_version: String,
    pub instance_revision_id: Uuid,
}

#[derive(FromRow)]
pub struct RunView {
    pub id: Uuid,
    pub state: String,
    pub outcome: Option<String>,
    pub exit_code: Option<i32>,
    pub exit_signal: Option<i32>,
    pub failure: Option<String>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
    pub state_version: i64,
    pub agent_id: Uuid,
    pub agent_name: String,
    pub instance_project_id: Uuid,
    pub instance_project_name: String,
    pub instance_revision_id: Uuid,
    pub release_id: Uuid,
    pub release_version: String,
    pub source_repository_id: Uuid,
    pub repository_id: Uuid,
    pub repository_name: String,
    pub project_id: Uuid,
    pub project_name: String,
    pub organization_id: Uuid,
    pub organization_name: String,
    pub input_commit: String,
    pub git_ref: String,
    pub attempt: i32,
    pub retry_supported: bool,
    pub result_id: Option<Uuid>,
    pub result_commit: Option<String>,
    pub result_ref: Option<String>,
    pub result_tree: Option<String>,
    pub result_message: Option<String>,
    pub artifact_manifest_hash: Option<String>,
    pub proposal_id: Option<Uuid>,
    pub proposal_state: Option<String>,
    pub proposal_target_ref: Option<String>,
    pub proposal_version: Option<i64>,
    pub events: Vec<RunEvent>,
    pub artifacts: Vec<RunArtifact>,
    pub patch_preview: Option<String>,
    pub manifest_preview: Option<String>,
}

pub struct RunEvent {
    pub sequence: i64,
    pub event_type: String,
    pub payload: EventPayload,
    pub occurred_at: OffsetDateTime,
}
pub enum EventPayload {
    Log(String),
    Metric {
        name: String,
        value: f64,
        labels: BTreeMap<String, String>,
    },
    State(String),
}

#[derive(FromRow)]
pub struct RunArtifact {
    pub id: Uuid,
    pub kind: String,
    pub path: String,
    pub mode: Option<i32>,
    pub media_type: Option<String>,
    pub size_bytes: i64,
    pub sha256: String,
    pub(crate) storage_key: String,
}

pub enum ControlKind {
    Cancel,
    Retry,
    Approve,
    Reject,
}
pub enum ControlTarget {
    Run(Uuid),
    Proposal(Uuid),
}
pub struct RequestControl {
    pub kind: ControlKind,
    pub repository_id: Uuid,
    pub target: ControlTarget,
    pub reason: String,
}
pub struct RequestedControl {
    pub id: Uuid,
    pub state: String,
}

pub struct RunApplication {
    pub(super) pool: PgPool,
    pub(super) result_artifact_root: PathBuf,
}

/// Immutable launch contract loaded for a run before guest construction.
#[derive(FromRow)]
pub struct VmLaunchContract {
    pub runtime_contract: Value,
    pub effective_runtime_policy: Value,
    pub requires_state: bool,
    pub update_hook: Option<Value>,
    pub release_state: String,
    pub revision_runnable: bool,
    pub attachment_runnable: bool,
    pub agent_update_id: Option<Uuid>,
}
