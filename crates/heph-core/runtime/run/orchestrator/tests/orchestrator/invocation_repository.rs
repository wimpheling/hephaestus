//! Every required persistence entry point panics: rejection must precede reads too.

use async_trait::async_trait;
use run_domain::{CancelRun, Run, RunState, StartRun};
use run_orchestrator::{CreateRunResult, RepositoryError, RunRepository, StoredVmEvent};
use runtime_types::{LeaseId, RunId, VolumeId};
use vm_trait::VmExit;

pub(super) struct NoIoRepository;

#[async_trait]
impl RunRepository for NoIoRepository {
    async fn create_run(&self, _: &StartRun) -> Result<CreateRunResult, RepositoryError> {
        panic!("Invocation reached create_run")
    }
    async fn ensure_runtime_git_provenance(&self, _: &Run) -> Result<(), RepositoryError> {
        panic!("Invocation reached Git provenance")
    }
    async fn get(&self, _: RunId) -> Result<Run, RepositoryError> {
        panic!("Invocation reached get")
    }
    async fn bind_resources(
        &self,
        _: RunId,
        _: Option<VolumeId>,
        _: Option<LeaseId>,
        _: Option<i64>,
        _: &str,
    ) -> Result<Run, RepositoryError> {
        panic!("Invocation reached bind_resources")
    }
    async fn transition(
        &self,
        _: RunId,
        _: RunState,
        _: Option<&VmExit>,
        _: Option<&str>,
    ) -> Result<Run, RepositoryError> {
        panic!("Invocation reached transition")
    }
    async fn append_vm_event(&self, _: RunId, _: StoredVmEvent) -> Result<(), RepositoryError> {
        panic!("Invocation reached append_vm_event")
    }
    async fn request_cancel(&self, _: &CancelRun) -> Result<bool, RepositoryError> {
        panic!("Invocation reached request_cancel")
    }
    async fn recoverable_runs(&self) -> Result<Vec<Run>, RepositoryError> {
        panic!("Invocation reached recoverable_runs")
    }
}

pub(super) struct NoIoSpecFactory;

#[async_trait]
impl run_orchestrator::VmSpecFactory for NoIoSpecFactory {
    async fn build(&self, _: &Run) -> Result<vm_trait::VmSpec, vm_trait::VmError> {
        panic!("Invocation reached VM specification factory")
    }
}
