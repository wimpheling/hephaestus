//! Durable run orchestration over provider-neutral ports.

mod orchestrator;
mod repository;
mod runtime_catalog;

pub use orchestrator::{
    CompositeRunCompletionObserver, OrchestratorError, PreparedRunAuthority, PreparedRunRuntime,
    PreparedRunSecrets, RunAuthorityError, RunAuthorityManager, RunAuthorizationError,
    RunCompletionError, RunCompletionObserver, RunLaunchAuthorizer, RunOperationGuards,
    RunOrchestrator, RunResourceObservationError, RunResourceObserver, RunRuntimeError,
    RunRuntimeManager, RunSecretError, RunSecretManager, VmSpecFactory,
};
pub use repository::{
    CreateRunResult, RepositoryError, RunCleanupRepository, RunRepository, StoredVmEvent,
};
pub use runtime_catalog::{
    MailboxRuntimeEvent, RunRuntimeArtifact, RunRuntimeArtifactKind, RunRuntimeCatalog,
    RunRuntimeCatalogError, RunRuntimeInput,
};
