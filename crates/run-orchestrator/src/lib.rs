//! Durable run orchestration, `PostgreSQL` persistence, and `JetStream` delivery.

mod nats;
mod orchestrator;
mod repository;

pub use nats::{
    CANCEL_RUN_SUBJECT, CommandConsumerError, CommandHandlingError, LIFECYCLE_EVENT_SUBJECT,
    NatsCommandHandler, NatsOutboxPublisher, OutboxPublishError, START_RUN_SUBJECT, TopologyError,
    ensure_jetstream_topology,
};
pub use orchestrator::{OrchestratorError, RunOrchestrator, VmSpecFactory};
pub use repository::{
    CreateRunResult, OutboxRecord, PgRunRepository, RepositoryError, RunRepository, StoredVmEvent,
};
