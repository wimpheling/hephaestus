use async_trait::async_trait;
use run_domain::{Run, RunState};
use run_orchestrator::{RunOrchestrator, VmSpecFactory};
use std::sync::Arc;
use vm_trait::{VmError, VmSpec};

use super::super::support::TestSpecFactory;
use super::Fixture;

struct MalformedTimeout;

#[async_trait]
impl VmSpecFactory for MalformedTimeout {
    async fn build(&self, run: &Run) -> Result<VmSpec, VmError> {
        let mut spec = TestSpecFactory.build(run).await?;
        spec.labels.insert(
            "hephaestus.wall-clock-timeout-seconds".into(),
            "invalid".into(),
        );
        Ok(spec)
    }
}

#[tokio::test]
async fn unexpected_primary_error_is_returned_even_after_proven_cleanup() {
    let fixture = Fixture::new();
    fixture.add_leases(2);
    let orchestrator = RunOrchestrator::new(
        fixture.runs.clone(),
        fixture.volumes.clone(),
        fixture.provider.clone(),
        Arc::new(MalformedTimeout),
        32 * 1024 * 1024,
    )
    .with_cleanup_repository(
        fixture.cleanup.clone(),
        Arc::new(super::volumes::GlobalVolumes(fixture.volumes.clone())),
    );
    let error = orchestrator.start_run(&fixture.command).await.unwrap_err();
    assert!(error.to_string().contains("must be a positive integer"));
    assert_eq!(fixture.runs.run.lock().await.state, RunState::CleanedUp);
    assert_eq!(fixture.count("provision"), 0);
    assert_eq!(fixture.count("finish-all"), 1);
}
