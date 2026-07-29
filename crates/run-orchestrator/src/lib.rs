//! Durable run orchestration, `PostgreSQL` persistence, and `JetStream` delivery.

mod nats;
mod orchestrator;
mod repository;

pub use nats::{
    CANCEL_RUN_SUBJECT, CommandConsumerError, CommandHandlingError, FORGE_START_RUN_SUBJECT,
    LIFECYCLE_EVENT_SUBJECT, NatsCommandHandler, NatsOutboxPublisher, OutboxPublishError,
    START_RUN_SUBJECT, TopologyError, ensure_jetstream_topology,
};
pub use orchestrator::{
    OrchestratorError, PreparedRunRuntime, PreparedRunSecrets, RunAuthorizationError,
    RunCompletionError, RunCompletionObserver, RunLaunchAuthorizer, RunOrchestrator,
    RunRuntimeError, RunRuntimeManager, RunSecretError, RunSecretManager, VmSpecFactory,
};
pub use repository::{
    CreateRunResult, OutboxRecord, PgRunRepository, RepositoryError, RunRepository, StoredVmEvent,
};
