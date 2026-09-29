//! `PostgreSQL` forge metadata adapter.
//!
//! PostgreSQL-backed repository metadata and receive units of work.

mod repository;

use async_trait::async_trait;
use forge_service::ForgeOutboxStore;
pub use forge_service::{
    CreateRepository, ForgeRepositoryError, OutboxRecord, ReceiveResult, RunRequest,
};
pub use forge_service::{GitStorage, GitStorageError};
pub use repository::PgForgeRepository;
use runtime_types::EventId;

#[async_trait]
impl ForgeOutboxStore for PgForgeRepository {
    async fn unpublished_outbox(
        &self,
        limit: i64,
    ) -> Result<Vec<OutboxRecord>, ForgeRepositoryError> {
        Self::unpublished_outbox(self, limit).await
    }

    async fn mark_outbox_published(&self, id: EventId) -> Result<(), ForgeRepositoryError> {
        Self::mark_outbox_published(self, id).await
    }

    async fn mark_outbox_failed(
        &self,
        id: EventId,
        error: &str,
    ) -> Result<(), ForgeRepositoryError> {
        Self::mark_outbox_failed(self, id, error).await
    }
}

/// Durable isolated-build request subject.
pub const BUILD_REQUESTED_SUBJECT: &str = "hephaestus.build.requested.v1";
/// Durable reusable-instance run request subject.
pub const INSTANCE_RUN_REQUESTED_SUBJECT: &str = "hephaestus.instance.run.requested.v1";
/// Durable run-start command subject.
pub const RUN_START_SUBJECT: &str = "hephaestus.run.start";
