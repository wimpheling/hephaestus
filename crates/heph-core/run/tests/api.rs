//! Compile-time coverage for the approved `heph-run` facade surface.

use heph_run::{
    CancelRun, CompositeRunCompletionObserver, OrchestratorError, PreparedRunAuthority,
    RepositoryError, Run, RunAuthorityError, RunAuthorityManager, RunCompletionError,
    RunCompletionObserver, RunKind, RunOrchestrator, RunOutcome, RunRepository, RunRuntimeArtifact,
    RunRuntimeArtifactKind, RunRuntimeCatalog, RunRuntimeCatalogError, RunRuntimeInput,
    RunSecretManager, RunState, StartRun, VmSpecFactory,
};
use std::sync::Arc;

fn stable_contracts(
    _run: &Run,
    _start: &StartRun,
    _cancel: &CancelRun,
    _orchestrator: &RunOrchestrator,
    _repository: Arc<dyn RunRepository>,
    _catalog: Arc<dyn RunRuntimeCatalog>,
    _authority: Arc<dyn RunAuthorityManager>,
    _completion: Arc<dyn RunCompletionObserver>,
    _secrets: Arc<dyn RunSecretManager>,
    _factory: Arc<dyn VmSpecFactory>,
) -> Result<Run, OrchestratorError> {
    let _ = (
        RunKind::Normal,
        RunOutcome::Succeeded,
        RunState::Queued,
        PreparedRunAuthority::default(),
        RunRuntimeArtifactKind::File,
    );
    let _: fn(&Run) -> Result<(), RunAuthorityError> = |_| Ok(());
    let _: fn(&Run) -> Result<(), RunCompletionError> = |_| Ok(());
    let _: fn(&Run) -> Result<RunRuntimeInput, RunRuntimeCatalogError> =
        |_| Err(RunRuntimeCatalogError::Unavailable);
    let _: fn(&Run) -> Result<(), RepositoryError> = |_| Ok(());
    let _ = CompositeRunCompletionObserver::new(Vec::new());
    Ok(_run.clone())
}

#[test]
fn approved_facade_contracts_compile() {
    let _: fn(
        &Run,
        &StartRun,
        &CancelRun,
        &RunOrchestrator,
        Arc<dyn RunRepository>,
        Arc<dyn RunRuntimeCatalog>,
        Arc<dyn RunAuthorityManager>,
        Arc<dyn RunCompletionObserver>,
        Arc<dyn RunSecretManager>,
        Arc<dyn VmSpecFactory>,
    ) -> Result<Run, OrchestratorError> = stable_contracts;
    let _: fn() -> PreparedRunAuthority = PreparedRunAuthority::default;
    let _: fn() -> CompositeRunCompletionObserver =
        || CompositeRunCompletionObserver::new(Vec::new());
    let _ = std::mem::size_of::<RunRuntimeArtifact>();
    let _ = std::mem::size_of::<RunRuntimeArtifactKind>();
}
