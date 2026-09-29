//! Validated VM specification construction for gateway services.

use super::{GatewayEdgeError, GatewayRuntimeContract};
use gateway_domain::{GatewayServiceConfig, GatewayServiceIdentity, service_transport_spec};
use std::{collections::BTreeMap, path::PathBuf};
use vm_trait::{GuestCommand, NetworkMode, RootFilesystem, VmId, VmMount, VmResources, VmSpec};

pub fn service_vm_spec(
    identity: GatewayServiceIdentity,
    service: &GatewayServiceConfig,
    contract: GatewayRuntimeContract,
    root: RootFilesystem,
    mounts: Vec<VmMount>,
) -> Result<VmSpec, GatewayEdgeError> {
    let has_release_mount = mounts
        .iter()
        .filter(|mount| mount.guest_path == PathBuf::from("/release"))
        .count()
        == 1;
    let has_control_mount = mounts
        .iter()
        .filter(|mount| mount.guest_path == PathBuf::from("/run/hephaestus"))
        .count()
        == 1;
    if mounts.len() != 2
        || mounts.iter().any(|mount| !mount.read_only)
        || !has_release_mount
        || !has_control_mount
    {
        return Err(GatewayEdgeError::HandlerUnavailable);
    }
    Ok(VmSpec {
        id: VmId(format!("gateway-service-{}", identity.instance_id)),
        root,
        disks: Vec::new(),
        mounts,
        resources: VmResources {
            vcpus: contract.policy_ceiling.vcpus,
            memory_mib: contract.policy_ceiling.memory_mib,
        },
        network: NetworkMode::Disabled,
        private_http_service: Some(service_transport_spec(service)),
        command: GuestCommand {
            program: format!("/release/{}", contract.command),
            args: contract.arguments,
            env: BTreeMap::new(),
            working_dir: Some(PathBuf::from(format!(
                "/release/{}",
                contract.working_directory
            ))),
        },
        runtime_authority: None,
        runtime_git_bridge: None,
        labels: BTreeMap::from([
            (
                String::from("hephaestus.kind"),
                String::from("gateway-service"),
            ),
            (
                String::from("hephaestus.gateway"),
                identity.gateway_id.to_string(),
            ),
            (
                String::from("hephaestus.gateway-revision"),
                identity.revision_id.to_string(),
            ),
            (
                String::from("hephaestus.gateway-instance"),
                identity.instance_id.to_string(),
            ),
        ]),
    })
}
