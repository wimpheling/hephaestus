//! Durable run orchestration over provider-neutral ports.

mod cleanup_wake;
mod command_executor;
mod invocation_cancel;
mod orchestrator;
mod repository;
mod runtime_catalog;

pub use cleanup_wake::{
    RunCleanupWakeBinding, RunCleanupWakeOriginal, RunCleanupWakeProgress, RunCleanupWakeRequest,
    RunCleanupWakeSnapshot,
};
pub use command_executor::RunCommandExecutor;
pub use invocation_cancel::{
    InvocationCancelEvidence, InvocationCancelPreparation, PreparedInvocationCancellation,
};

pub use orchestrator::{
    CompositeRunCompletionObserver, OrchestratorError, PreparedRunAuthority, PreparedRunRuntime,
    PreparedRunSecrets, RunAuthorityError, RunAuthorityManager, RunAuthorizationError,
    RunCompletionError, RunCompletionObserver, RunLaunchAuthorizer, RunOperationGuards,
    RunOrchestrator, RunResourceObservationError, RunResourceObserver, RunRuntimeError,
    RunRuntimeManager, RunSecretError, RunSecretManager, SupervisorReconciliationCause,
    VmSpecFactory,
};
pub use repository::{
    CreateRunResult, RepositoryError, RunCleanupRepository, RunRepository, StoredVmEvent,
};
pub use runtime_catalog::{
    MailboxRuntimeEvent, RunRuntimeArtifact, RunRuntimeArtifactKind, RunRuntimeCatalog,
    RunRuntimeCatalogError, RunRuntimeInput,
};
