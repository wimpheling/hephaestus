use serde_json::Value;
use sqlx::FromRow;
use time::OffsetDateTime;
use uuid::Uuid;

/// Bounded stable UUID cursor shared by project list operations.
#[derive(Clone, Copy)]
pub struct Page {
    pub size: i64,
    pub after: Option<Uuid>,
}

/// Project application failures.
#[derive(Debug, thiserror::Error)]
pub enum ProjectError {
    #[error("project access is denied")]
    PermissionDenied,
    #[error("project was not found")]
    NotFound,
    #[error("project image cannot be retried in its current state")]
    Conflict,
    #[error("project query failed")]
    Persistence(#[source] sqlx::Error),
    #[error("project page is invalid")]
    InvalidPage,
}

#[derive(FromRow)]
pub struct ProjectRow {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub organization_id: Uuid,
    pub organization_name: String,
}

#[derive(FromRow)]
pub struct ProjectRepositoryRow {
    pub id: Uuid,
    pub name: String,
    pub default_branch: String,
    pub is_public: bool,
    pub attachment_count: i64,
    pub run_count: i64,
}

/// Redacted project-owned repository OCI image projection.
#[derive(FromRow)]
pub struct ProjectRepositoryImageRow {
    pub id: Uuid,
    pub repository_id: Uuid,
    pub key: String,
    pub display_name: String,
    pub source_revision: String,
    pub base_image_reference: String,
    pub status: String,
    pub image_reference: Option<String>,
    pub failure_reason: Option<String>,
    pub updated_at: OffsetDateTime,
}

/// One bounded redacted preparation transition for a project image.
#[derive(FromRow)]
pub struct ProjectRepositoryImagePreparationEventRow {
    pub id: Uuid,
    pub phase: String,
    pub outcome: String,
    pub output_digest: Option<String>,
    pub safe_reason: Option<String>,
    pub occurred_at: OffsetDateTime,
}

#[derive(FromRow)]
pub struct InstanceRow {
    pub id: Uuid,
    pub name: String,
    pub state: String,
    pub run_gate_open: bool,
    pub active_revision_id: Uuid,
    pub state_volume_id: Option<Uuid>,
    pub updated_at: OffsetDateTime,
    pub runnable: Option<bool>,
    pub platform_policy_version: Option<String>,
    pub diagnostics: Option<Value>,
    pub release_id: Option<Uuid>,
    pub release_version: Option<String>,
    pub release_state: Option<String>,
    pub release_agent_name: Option<String>,
    pub attachment_count: i64,
    pub run_count: i64,
    pub last_run_at: Option<OffsetDateTime>,
    pub mailbox_id: Option<Uuid>,
}

#[derive(FromRow)]
pub struct ReleaseAgentRow {
    pub id: Uuid,
    pub display_name: String,
    pub parameter_schema: Value,
    pub secret_slot_schema: Value,
    pub runtime_contract: Value,
    pub requires_state: bool,
    pub release_id: Uuid,
    pub release_version: String,
    pub source_commit: String,
    pub repository_id: Uuid,
    pub repository_name: String,
    pub capability_requirements: Value,
}

pub struct PageResult<T> {
    pub values: Vec<T>,
    pub next: Option<String>,
}
