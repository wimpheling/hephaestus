//! Compile-time coverage for the approved `heph-run` facade surface.

use heph_run::{
    CancelRun, CompositeRunCompletionObserver, OrchestratorError, PreparedRunAuthority,
    RepositoryError, Run, RunAuthorityError, RunAuthorityManager, RunCompletionError,
    RunCompletionObserver, RunKind, RunOrchestrator, RunOutcome, RunRepository, RunRuntimeArtifact,
    RunRuntimeArtifactKind, RunRuntimeCatalog, RunRuntimeCatalogError, RunRuntimeInput,
    RunSecretManager, RunState, StartRun, VmSpecFactory,
};
use std::sync::Arc;

const fn assert_type<T>() {}

#[test]
fn approved_facade_contracts_compile() {
    assert_type::<Run>();
    assert_type::<StartRun>();
    assert_type::<CancelRun>();
    assert_type::<RunOrchestrator>();
    assert_type::<Arc<dyn RunRepository>>();
    assert_type::<Arc<dyn RunRuntimeCatalog>>();
    assert_type::<Arc<dyn RunAuthorityManager>>();
    assert_type::<Arc<dyn RunCompletionObserver>>();
    assert_type::<Arc<dyn RunSecretManager>>();
    assert_type::<Arc<dyn VmSpecFactory>>();
    assert_type::<OrchestratorError>();
    assert_type::<RepositoryError>();
    assert_type::<RunAuthorityError>();
    assert_type::<RunCompletionError>();
    assert_type::<RunRuntimeCatalogError>();
    assert_type::<RunRuntimeInput>();
    assert_type::<RunRuntimeArtifact>();
    assert_type::<RunRuntimeArtifactKind>();
    assert_type::<RunKind>();
    assert_type::<RunOutcome>();
    assert_type::<RunState>();
    let _ = PreparedRunAuthority::default();
    let _ = CompositeRunCompletionObserver::new(Vec::new());
}
