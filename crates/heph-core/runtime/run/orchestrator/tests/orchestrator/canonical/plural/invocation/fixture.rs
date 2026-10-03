use run_domain::{RunCleanupHostId, RunKind};
use run_orchestrator::{RunOrchestrator, VmSpecFactory};
use std::{
    sync::{Arc, atomic::Ordering},
    time::Duration,
};
use vm_trait::{VmId, VmProvider};

use super::{
    super::{Authorizer, Factory, Fixture, Store, orchestrator},
    ports::{ControlledProvider, NoGit},
};

pub async fn fixture() -> Fixture {
    let mut fixture = Fixture::new();
    fixture.command.kind = RunKind::Invocation;
    fixture.command.attachment_id = None;
    let mut run = fixture.runs.run.lock().await;
    run.kind = RunKind::Invocation;
    run.attachment_id = None;
    run.vm_id = Some(fixture.command.run_id.to_string());
    drop(run);
    let scope = fixture.provider.owner_scope().unwrap();
    *fixture.runs.planned_vm.lock().await = Some((
        RunCleanupHostId::new(scope.namespace().into(), scope.host_id().into()).unwrap(),
        VmId(fixture.command.run_id.to_string()),
    ));
    *fixture.runs.created.lock().await = true;
    fixture
        .runs
        .invocation_admitted
        .store(true, Ordering::SeqCst);
    fixture
}

pub fn configured(
    fixture: &Fixture,
    count: usize,
    provider: Arc<dyn VmProvider>,
    factory: Arc<dyn VmSpecFactory>,
) -> RunOrchestrator {
    with_authorizer(
        fixture,
        count,
        provider,
        factory,
        Arc::new(Authorizer::default()),
    )
}

pub fn with_authorizer(
    fixture: &Fixture,
    count: usize,
    provider: Arc<dyn VmProvider>,
    factory: Arc<dyn VmSpecFactory>,
    authorizer: Arc<Authorizer>,
) -> RunOrchestrator {
    orchestrator(
        fixture,
        Arc::new(Store::new(fixture, count)),
        authorizer,
        provider,
        factory,
    )
    .with_qualified_invocation(fixture.provider.owner_scope().unwrap())
    .with_workspace_manager(Arc::new(NoGit))
    .with_runtime_git_workspace_manager(Arc::new(NoGit))
}

pub fn controlled(
    fixture: &Fixture,
    count: usize,
) -> (Arc<RunOrchestrator>, Arc<ControlledProvider>) {
    let provider = Arc::new(ControlledProvider::new(fixture.provider.clone()));
    (
        Arc::new(configured(
            fixture,
            count,
            provider.clone(),
            Arc::new(Factory),
        )),
        provider,
    )
}

pub async fn wait_finalize(fixture: &Fixture) {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let notified = fixture.runs.event_entered.notified();
            if fixture
                .runs
                .events
                .lock()
                .await
                .iter()
                .any(|event| event.event_type == "vm.finalize_result")
            {
                return;
            }
            notified.await;
        }
    })
    .await
    .expect("controlled finalize must be persisted");
}
