//! Stable provider-neutral contracts and operations for durable runs.
//!
//! `PostgreSQL` repositories and broker-specific command handlers remain in
//! their leaf crates. This facade intentionally exposes only the contracts
//! needed by composition and application code.
//!
//! The facade does not expose persistence adapters:
//!
//! ```compile_fail
//! use heph_run::PgRunRepository;
//! ```

pub use run_domain::{CancelRun, Run, RunKind, RunOutcome, RunState, StartRun};
pub use run_orchestrator::{
    CompositeRunCompletionObserver, OrchestratorError, PreparedRunAuthority, RepositoryError,
    RunAuthorityError, RunAuthorityManager, RunCompletionError, RunCompletionObserver,
    RunOrchestrator, RunRepository, RunRuntimeArtifact, RunRuntimeArtifactKind, RunRuntimeCatalog,
    RunRuntimeCatalogError, RunRuntimeInput, RunSecretManager, VmSpecFactory,
};
