//! Durable forge persistence, bare repository storage, and receive processing.

mod nats;
mod repository;
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
