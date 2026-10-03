//! Complete-set core semantics with fake trusted ports; no native dispatch claim.
#[path = "plural/drain.rs"]
mod drain;
#[path = "plural/invocation.rs"]
mod invocation;
#[path = "plural/monitor.rs"]
mod monitor;
#[path = "plural/ports.rs"]
mod ports;
#[path = "plural/preparation.rs"]
mod preparation;
#[path = "plural/stalled.rs"]
mod stalled;
#[path = "plural/store.rs"]
mod store;

use super::Fixture;
use ports::{Authorizer, Factory};
use run_orchestrator::{RunOrchestrator, VmSpecFactory};
use std::sync::Arc;
use store::Store;
use vm_trait::VmProvider;

fn orchestrator(
    fixture: &Fixture,
    store: Arc<Store>,
    authorizer: Arc<Authorizer>,
    provider: Arc<dyn VmProvider>,
    factory: Arc<dyn VmSpecFactory>,
) -> RunOrchestrator {
    RunOrchestrator::new(
        fixture.runs.clone(),
        fixture.volumes.clone(),
        provider,
        factory,
        32 * 1024 * 1024,
    )
    .with_cleanup_repository(fixture.cleanup.clone(), store)
    .with_launch_authorizer(authorizer)
    .with_volume_preparation()
    .with_completion_observer(Arc::new(super::super::support::RecordingCompletion {
        log: fixture.log.clone(),
    }))
}
