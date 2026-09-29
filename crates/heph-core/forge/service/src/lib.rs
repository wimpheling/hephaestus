//! Forge persistence.

mod nats;
// Rust 1.85 Clippy misidentifies thiserror's attribute formatting as an
// unexpanded formatting literal in this module.
#[allow(clippy::literal_string_with_formatting_args)]
mod storage;

pub use nats::{
    BUILD_REQUESTED_SUBJECT, BUILD_RETRY_REQUESTED_SUBJECT, BUILD_VERIFY_REQUESTED_SUBJECT,
    ForgeNatsOutboxPublisher, ForgeOutboxPublishError, INSTANCE_RUN_REQUESTED_SUBJECT,
    RUN_START_SUBJECT, ensure_build_consumer, ensure_forge_jetstream_topology,
};
pub use storage::{GitStorage, GitStorageError};

use async_trait::async_trait;
use forge_domain::{CommitSha, GitRef, ProjectId, ReceiveId, RepositoryId, RunRequestId};
use release_domain::BuildRequestId;
use run_domain::StartRun;
use runtime_types::EventId;
use serde_json::Value;
use std::error::Error;

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

/// Durable run request emitted by receive processing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunRequest {
    /// Stable request identifier.
    pub id: RunRequestId,
    /// Repository containing the accepted update.
    pub repository_id: RepositoryId,
    /// Exact received commit.
    pub commit_sha: CommitSha,
    /// Exact updated ref.
    pub git_ref: GitRef,
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
    /// Idempotently created isolated-build requests.
    pub build_requests: Vec<BuildRequestId>,
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
    /// Serialized message payload.
    pub payload: Value,
}

/// Provider-neutral forge persistence failure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ForgeRepositoryError {
    /// Repository metadata does not exist.
    #[error("repository {0} was not found")]
    RepositoryNotFound(RepositoryId),
    /// Receive identifier was reused for another repository.
    #[error("receive identifier {0} conflicts with an existing receive")]
    ReceiveConflict(ReceiveId),
    /// Caller metadata is invalid.
    #[error("invalid forge metadata: {0}")]
    InvalidMetadata(&'static str),
    /// Stored data violates domain invariants.
    #[error("invalid stored forge data in {0}")]
    InvalidStoredData(&'static str),
    /// Authorization denied.
    #[error("forge command is not authorized")]
    AuthorizationDenied,
    /// Authorization provider unavailable.
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
    /// Persistence provider failed.
    #[error("forge persistence failed: {0}")]
    Storage(#[source] Box<dyn Error + Send + Sync>),
}

/// SQL-free durable outbox port used by the NATS publisher.
#[async_trait]
pub trait ForgeOutboxStore: Send + Sync {
    /// Returns pending forge messages.
    async fn unpublished_outbox(
        &self,
        limit: i64,
    ) -> Result<Vec<OutboxRecord>, ForgeRepositoryError>;
    /// Marks one message published.
    async fn mark_outbox_published(&self, id: EventId) -> Result<(), ForgeRepositoryError>;
    /// Records a publication error.
    async fn mark_outbox_failed(
        &self,
        id: EventId,
        error: &str,
    ) -> Result<(), ForgeRepositoryError>;
}
