//! Authorized release read operations used by the transport layer.
#![allow(clippy::unused_async)] // Query methods retain async transport contracts while adapter SQL is introduced.

use crate::build::BuildView;
use agent_config::SecretSlotDeclaration;
use identity_domain::AuthenticatedIdentity;
use release_domain::{ParameterDeclaration, UpdateHook};
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Clone, Copy)]
pub struct ReleasePage {
    pub size: i64,
    pub after: Option<Uuid>,
}

pub type ReleaseState = release_domain::ReleaseState;

pub fn encode_cursor(value: Uuid) -> String {
    value.to_string()
}
pub fn decode_cursor(value: &str) -> Option<Uuid> {
    Uuid::parse_str(value).ok()
}

pub struct ReleasePageResult {
    pub releases: Vec<ReleaseSummary>,
    pub next: Option<Uuid>,
}

pub struct ReleaseSummary {
    pub id: Uuid,
    pub version: String,
    pub state: ReleaseState,
    pub source_commit: String,
    pub source_ref: String,
    pub build_request_id: Uuid,
    pub created_at: OffsetDateTime,
    pub published_at: Option<OffsetDateTime>,
    pub manifest_hash: String,
    pub artifact_count: u32,
    pub agent_count: u32,
}

pub struct ReleaseArtifact {
    pub id: Uuid,
    pub path: String,
    pub kind: String,
    pub mode: u32,
    pub sha256: String,
    pub size_bytes: u64,
    pub media_type: String,
}
pub struct ReleaseAgent {
    pub id: Uuid,
    pub family_id: Uuid,
    pub agent_key: String,
    pub display_name: String,
    pub policy: release_domain::RuntimePolicy,
    pub requires_state: bool,
    pub parameter_schema: Vec<ParameterDeclaration>,
    pub secret_slots: Vec<SecretSlotDeclaration>,
    pub update_hook: Option<UpdateHook>,
    pub created_at: OffsetDateTime,
}
pub struct ReleaseDetail {
    pub summary: ReleaseSummary,
    pub build_definition_hash: String,
    pub configuration_hash: String,
    pub revoked_at: Option<OffsetDateTime>,
    pub repository_id: Uuid,
    pub repository_name: String,
    pub project_id: Uuid,
    pub project_name: String,
    pub organization_id: Uuid,
    pub organization_name: String,
    pub build: BuildView,
    pub artifacts: Vec<ReleaseArtifact>,
    pub agents: Vec<ReleaseAgent>,
}

#[derive(Debug, thiserror::Error)]
pub enum ReleaseError {
    #[error("release not found")]
    NotFound,
    #[error("invalid page")]
    InvalidPage,
    #[error("invalid stored data")]
    InvalidStoredData,
    #[error("serialization failed: {0}")]
    Serialization(#[source] serde_json::Error),
    #[error("persistence failed: {0}")]
    Persistence(#[source] sqlx::Error),
}

pub struct ReleaseApplication {
    pool: PgPool,
}
impl ReleaseApplication {
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }
    pub async fn list_repository_releases(
        &self,
        _identity: &AuthenticatedIdentity,
        _repository_id: Uuid,
        _page: ReleasePage,
    ) -> Result<ReleasePageResult, ReleaseError> {
        let _ = &self.pool;
        Err(ReleaseError::NotFound)
    }
    pub async fn get_release(
        &self,
        _identity: &AuthenticatedIdentity,
        _id: Uuid,
    ) -> Result<ReleaseDetail, ReleaseError> {
        let _ = &self.pool;
        Err(ReleaseError::NotFound)
    }
}
