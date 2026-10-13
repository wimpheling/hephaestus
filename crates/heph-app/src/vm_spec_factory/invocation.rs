//! Explicit qualified projection; supplied kind and constructor inputs grant no IO.

#[path = "invocation/projection.rs"]
mod projection;

use async_trait::async_trait;
use heph_run::{RunRepository, RunState};
use run_orchestrator::{RunRuntimeCatalog, RunRuntimeInput};
use std::sync::Arc;
use volume_trait::RunVolumeStore;

use super::{
    PgAgentVmSpecFactory, Run, RunKind, RunVolumeSelections, VmError, VmSpec, VmSpecFactory,
    invalid_spec, load_vm_launch_contract, vm_factory_error, volumes,
};

pub(super) struct QualifiedInvocationFactory {
    ordinary: PgAgentVmSpecFactory,
    runs: Arc<dyn RunRepository>,
    catalog: Arc<dyn RunRuntimeCatalog>,
    volumes: Arc<dyn RunVolumeStore>,
}

impl QualifiedInvocationFactory {
    pub(super) fn new(
        ordinary: PgAgentVmSpecFactory,
        runs: Arc<dyn RunRepository>,
        catalog: Arc<dyn RunRuntimeCatalog>,
        volumes: Arc<dyn RunVolumeStore>,
    ) -> Self {
        Self {
            ordinary,
            runs,
            catalog,
            volumes,
        }
    }

    async fn invocation_run(&self, supplied: &Run) -> Result<Option<Run>, VmError> {
        // Persisted kind precedes any scalar or ordinary delegation.
        let current = self.runs.get(supplied.id).await.map_err(vm_factory_error)?;
        if current.kind != RunKind::Invocation && supplied.kind != RunKind::Invocation {
            return Ok(None);
        }
        let vm_id = current.id.to_string();
        if &current != supplied
            || current.kind != RunKind::Invocation
            || current.state != RunState::Provisioning
            || current.vm_id.as_deref() != Some(vm_id.as_str())
            || current.attachment_id.is_some()
            || current.volume_id.is_some()
            || current.lease_id.is_some()
            || current.lease_fencing_token.is_some()
            || current.cancel_requested_at.is_some()
            || current.outcome.is_some()
            || current.exit.is_some()
            || current.failure.is_some()
        {
            return Err(denied("exact persisted Provisioning Invocation required"));
        }
        Ok(Some(current))
    }

    async fn project(
        &self,
        run: &Run,
        supplied: Option<&RunVolumeSelections>,
    ) -> Result<VmSpec, VmError> {
        // Actual configured RunPG119 proves owner/config/closure and the current
        // frozen held set, which are absent from the Run DTO. A trait object or
        // kind cannot prove that startup supplied the required implementation.
        let input = self
            .catalog
            .load_runtime(run)
            .await
            .map_err(vm_factory_error)?;
        validate_input(&input)?;
        let selected = self
            .volumes
            .load_run_selections(run.id)
            .await
            .map_err(vm_factory_error)?;
        if supplied.is_some_and(|expected| expected != &selected) {
            return Err(denied(
                "supplied Invocation selections differ from the full stored set",
            ));
        }
        self.volumes
            .preflight_run(&selected)
            .await
            .map_err(vm_factory_error)?;
        let stored = load_vm_launch_contract(&self.ordinary.pool, run.id.as_uuid())
            .await
            .map_err(vm_factory_error)?
            .ok_or_else(|| denied("exact Invocation release projection is unavailable"))?;
        projection::validate(run, &stored)?;
        volumes::validate(run, &stored, &selected)?;
        projection::build(&self.ordinary, run, stored, &input)
    }
}

#[async_trait]
impl VmSpecFactory for QualifiedInvocationFactory {
    async fn build(&self, supplied: &Run) -> Result<VmSpec, VmError> {
        match self.invocation_run(supplied).await? {
            Some(run) => self.project(&run, None).await,
            None => self.ordinary.build(supplied).await,
        }
    }

    async fn build_with_volumes(
        &self,
        supplied: &Run,
        selections: &RunVolumeSelections,
    ) -> Result<VmSpec, VmError> {
        match self.invocation_run(supplied).await? {
            Some(run) => self.project(&run, Some(selections)).await,
            None => self.ordinary.build_with_volumes(supplied, selections).await,
        }
    }
}

fn validate_input(input: &RunRuntimeInput) -> Result<(), VmError> {
    if !input.parameters.is_object()
        || input.artifacts.is_empty()
        || input.repository_id.is_some()
        || input.git_ref.is_some()
        || input.commit_sha.is_some()
        || input.update_id.is_some()
        || input.previous_revision_id.is_some()
        || input.previous_release_id.is_some()
        || input.previous_parameters.is_some()
        || !input.previous_artifacts.is_empty()
        || input.mailbox_event.is_some()
    {
        return Err(denied(
            "qualified Invocation catalogue contains contradictory runtime data",
        ));
    }
    Ok(())
}

fn denied(reason: &str) -> VmError {
    invalid_spec("invocation", reason)
}
