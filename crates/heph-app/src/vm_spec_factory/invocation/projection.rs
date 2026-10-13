//! Guest release paths retain the exact immutable executable and cwd semantics.

use control_plane_postgres::run::VmLaunchContract;
use release_domain::ArtifactPath;
use run_orchestrator::{RunRuntimeArtifactKind, RunRuntimeInput};

use super::super::{
    BTreeMap, GuestCommand, NetworkMode, StoredEffectivePolicy, StoredNetworkAccess,
    StoredRuntimeContract, VmResources, validate_runtime_policy, vm_factory_error,
};
use super::{PgAgentVmSpecFactory, Run, VmError, VmSpec, denied};

pub(super) fn validate(run: &Run, stored: &VmLaunchContract) -> Result<(), VmError> {
    if [
        stored.run_id,
        stored.instance_id,
        stored.instance_revision_id,
        stored.release_id,
        stored.release_agent_id,
    ] != [
        run.id.as_uuid(),
        run.instance_id.as_uuid(),
        run.instance_revision_id.as_uuid(),
        run.release_id.as_uuid(),
        run.release_agent_id.as_uuid(),
    ] || stored.requires_state != run.requires_state
        || stored.release_state != "published"
        || !stored.revision_runnable
        || stored.volume_mode != "named"
        || stored.state_volume_id.is_some()
        || stored.runtime_contract_hash.len() != 32
        || stored.agent_update_id.is_some()
        || stored.attachment_runnable
    {
        return Err(denied(
            "Invocation release projection contradicts its exact catalogue",
        ));
    }
    // attachment_runnable=false is expected ordinary-only projection DATA.
    // It is never rewritten or used instead of the prior positive119 proof.
    Ok(())
}

pub(super) fn build(
    factory: &PgAgentVmSpecFactory,
    run: &Run,
    stored: VmLaunchContract,
    input: &RunRuntimeInput,
) -> Result<VmSpec, VmError> {
    let contract: StoredRuntimeContract =
        serde_json::from_value(stored.runtime_contract).map_err(vm_factory_error)?;
    let executable = release_path(&contract.command)?;
    let working_directory = release_path(&contract.working_directory)?;
    if !input.artifacts.iter().any(|artifact| {
        artifact.path == contract.command && artifact.kind == RunRuntimeArtifactKind::Executable
    }) {
        return Err(denied(
            "released executable is absent from exact catalogue artifacts",
        ));
    }
    let policy: StoredEffectivePolicy =
        serde_json::from_value(stored.effective_runtime_policy).map_err(vm_factory_error)?;
    let root = factory
        .root_images
        .get(&contract.image_reference)
        .cloned()
        .ok_or_else(|| denied("immutable Invocation image is not materialized"))?;
    let resources = VmResources {
        vcpus: policy.vcpus,
        memory_mib: policy.memory_mib,
    };
    validate_runtime_policy(&factory.runtime_policy, &resources, policy.network)?;
    let network = match policy.network {
        StoredNetworkAccess::Disabled => NetworkMode::Disabled,
        StoredNetworkAccess::BrokerOnly => NetworkMode::BrokerOnly,
        StoredNetworkAccess::Egress => NetworkMode::UserMode {
            ingress: Vec::new(),
        },
    };
    Ok(VmSpec {
        id: heph_runtime::VmId(run.id.to_string()),
        root,
        disks: Vec::new(),
        mounts: Vec::new(),
        guest_volumes: Vec::new(),
        resources,
        network,
        command: GuestCommand {
            program: executable,
            args: contract.arguments,
            env: BTreeMap::new(),
            // This absolute GUEST cwd is not a host/workspace override. Core's
            // existing guard checks its exact /release spelling before provider IO.
            working_dir: Some(working_directory.into()),
        },
        runtime_authority: None,
        runtime_git_bridge: None,
        private_http_service: None,
        labels: BTreeMap::from([
            (
                "hephaestus.instance".to_owned(),
                run.instance_id.to_string(),
            ),
            (
                "hephaestus.instance-revision".to_owned(),
                run.instance_revision_id.to_string(),
            ),
            ("hephaestus.release".to_owned(), run.release_id.to_string()),
            (
                "hephaestus.platform-policy".to_owned(),
                factory.runtime_policy.version.clone(),
            ),
        ]),
    })
}

fn release_path(value: &str) -> Result<String, VmError> {
    let relative = ArtifactPath::parse(value).map_err(vm_factory_error)?;
    // Reject aliases hidden by Path component normalization (duplicate slashes,
    // interior dots, trailing slash), plus controls before the guest boundary.
    if value.chars().any(char::is_control)
        || value
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(denied("released guest path is not normalized"));
    }
    Ok(format!("/release/{}", relative.as_str()))
}
