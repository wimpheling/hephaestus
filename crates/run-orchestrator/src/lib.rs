//! Durable run orchestration and `JetStream` delivery over provider-neutral ports.

mod nats;
mod orchestrator;
mod repository;
mod runtime_catalog;

pub use nats::{
    CANCEL_RUN_SUBJECT, CommandConsumerError, CommandHandlingError, FORGE_START_RUN_SUBJECT,
    NatsCommandHandler, START_RUN_SUBJECT, TopologyError, ensure_jetstream_topology,
};
pub use orchestrator::{
    OrchestratorError, PreparedRunAuthority, PreparedRunRuntime, PreparedRunSecrets,
    RunAuthorityError, RunAuthorityManager, RunAuthorizationError, RunCompletionError,
    RunCompletionObserver, RunLaunchAuthorizer, RunOrchestrator, RunRuntimeError,
    RunRuntimeManager, RunSecretError, RunSecretManager, VmSpecFactory,
};
pub use repository::{CreateRunResult, RepositoryError, RunRepository, StoredVmEvent};
pub use runtime_catalog::{
    RunRuntimeArtifact, RunRuntimeArtifactKind, RunRuntimeCatalog, RunRuntimeCatalogError,
    RunRuntimeInput,
};
