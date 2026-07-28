//! Forge persistence.

mod nats;
mod repository;
// Rust 1.85 Clippy misidentifies thiserror's attribute formatting as an
// unexpanded formatting literal in this module.
#[allow(clippy::literal_string_with_formatting_args)]
mod storage;

pub use nats::{
    AGENT_CONFIG_INVALID_SUBJECT, ForgeNatsOutboxPublisher, ForgeOutboxPublishError,
    GIT_RECEIVE_ACCEPTED_SUBJECT, RUN_START_SUBJECT, ensure_forge_jetstream_topology,
};
pub use repository::{
    CreateRepository, ForgeRepositoryError, OutboxRecord, PgForgeRepository, ReceiveResult,
    RunRequest,
};
pub use storage::{GitStorage, GitStorageError};
