//! Opt-in durable cleanup ordering and conservative recovery with fake ports.

#[path = "canonical/atomic_creation.rs"]
mod atomic_creation;
#[path = "canonical/fixture.rs"]
mod fixture;
#[path = "canonical/provider.rs"]
mod provider;
#[path = "canonical/repository.rs"]
mod repository;
#[path = "canonical/volumes.rs"]
mod volumes;

#[path = "canonical/failures.rs"]
mod failures;
#[path = "canonical/primary_error.rs"]
mod primary_error;
#[path = "canonical/quiescence.rs"]
mod quiescence;
#[path = "canonical/recovery.rs"]
mod recovery;
#[path = "canonical/replay.rs"]
mod replay;

#[path = "canonical/plural.rs"]
mod plural;

use fixture::Fixture;
use run_domain::{RunCleanupHostId, RunState};
use run_orchestrator::RunCleanupRepository;
use vm_trait::VmId;

impl Fixture {
    async fn prepare_recovery(&self) {
        self.cleanup
            .bind_vm_before_provision(
                self.command.run_id,
                self.command.instance_id,
                self.command.instance_revision_id,
                &RunCleanupHostId::new("test-owner".into(), "test".into()).unwrap(),
                &VmId(self.command.run_id.to_string()),
            )
            .await
            .unwrap();
        self.runs.run.lock().await.state = RunState::Running;
        *self.runs.created.lock().await = true;
    }
}
