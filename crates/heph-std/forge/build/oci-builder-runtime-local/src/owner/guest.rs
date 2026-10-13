//! Actual original builder/verifier spec, retained token, raw VM outcomes.
use super::{Node, OciWorkerError, models::Phase};
use crate::{
    VerifiedVmOciOutput, VmOciOperation,
    constants::WorkloadPhaseTimer,
    filesystem::{symlinked_tree, verified_vm_output},
    guest::{guest_failure_phase, operation_phase},
    output::normalize_layout_to_single_index,
    specs::{builder_vm_spec, verifier_vm_spec},
};
use std::sync::Arc;
use vm_trait::VmSpec;
pub(super) async fn work(
    operation: &VmOciOperation,
    node: &Arc<Node>,
) -> Result<VerifiedVmOciOutput, OciWorkerError> {
    operation.custody.roots.recheck()?;
    node.open()?;
    let scratch = super::preparation::prepare(operation, node).await?;
    let candidate = node.pin(1)?;
    let verification = node.pin(2)?;
    operation.custody.roots.recheck()?;
    candidate.recheck()?;
    verification.recheck()?;
    let spec = builder_vm_spec(
        &node.request,
        candidate.path(),
        &scratch,
        &operation.builder_root,
        &operation.resources,
    )?;
    run(operation, node, Phase::Builder, "builder", spec).await?;
    candidate.recheck()?;
    if symlinked_tree(candidate.path())? {
        return Err(OciWorkerError::InvalidOutput);
    }
    normalize_layout_to_single_index(candidate.path())?;
    candidate.seal_candidate()?;
    let spec = verifier_vm_spec(
        &node.request,
        candidate.path(),
        verification.path(),
        &operation.verifier_root,
        &operation.resources,
    );
    run(operation, node, Phase::Verifier, "verifier", spec).await?;
    candidate.recheck()?;
    verification.recheck()?;
    operation.custody.roots.recheck()?;
    // Output remains original/pinned: publication/materialization is separate.
    verified_vm_output(candidate.path(), verification.path())
}
async fn run(
    operation: &VmOciOperation,
    node: &Arc<Node>,
    phase: Phase,
    name: &'static str,
    spec: VmSpec,
) -> Result<(), OciWorkerError> {
    let timing = match phase {
        Phase::Builder => "oci-builder",
        Phase::Verifier => "oci-verifier",
    };
    let timer = WorkloadPhaseTimer::start(timing, operation.workload_phase_timing);
    let result = run_inner(operation, node, phase, name, spec).await;
    timer.finish(result.is_ok());
    result
}
async fn run_inner(
    operation: &VmOciOperation,
    node: &Arc<Node>,
    phase: Phase,
    name: &'static str,
    spec: VmSpec,
) -> Result<(), OciWorkerError> {
    operation.custody.roots.recheck()?;
    node.pin(1)?.recheck()?;
    node.pin(2)?.recheck()?;
    if phase == Phase::Builder {
        node.pin(0)?.recheck()?;
    }
    let attempt = match operation.provider.prepare_provision_attempt(spec.clone()) {
        Ok(attempt) => attempt,
        Err(error) => {
            node.error(Arc::new(error));
            return Err(super::errors::failed(name));
        }
    };
    // Original opaque token/spec registered BEFORE any provision poll.
    node.install(phase, spec, attempt.clone())?;
    let instance = match attempt.provision().await {
        Ok(instance) => instance,
        Err(error) => {
            node.error(Arc::new(error));
            node.detach().await?;
            return Err(super::errors::failed(name));
        }
    };
    node.instance(phase, instance.clone())?;
    let mut events = instance.subscribe_events();
    node.open()?;
    if let Err(error) = instance.start().await {
        node.error(Arc::new(error));
        node.detach().await?;
        return Err(super::errors::failed(operation_phase(name, "startup")));
    }
    let exit = match instance.wait().await {
        Ok(exit) => Some(exit),
        Err(error) => {
            node.error(Arc::new(error));
            None
        }
    };
    let completed = exit
        .as_ref()
        .is_some_and(|exit| exit.code == Some(0) && exit.signal.is_none());
    if !completed {
        eprintln!(
            "isolated OCI {name} guest exit code={:?} signal={:?}",
            exit.as_ref().and_then(|value| value.code),
            exit.as_ref().and_then(|value| value.signal)
        );
    }
    let failure_phase = if completed {
        None
    } else {
        Some(guest_failure_phase(name, &mut events).await)
    };
    node.detach().await?;
    if !completed {
        return Err(OciWorkerError::IsolatedVmFailed {
            phase: failure_phase.unwrap_or_else(|| operation_phase(name, "execution")),
            exit_code: exit.and_then(|exit| exit.code),
        });
    }
    Ok(())
}
