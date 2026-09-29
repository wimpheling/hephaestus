use super::{
    helpers::invalid,
    types::{
        PreparedDisk, PreparedPrivateHttpService, PreparedRuntimeAuthority,
        PreparedRuntimeGitBridge,
    },
};
use crate::config::LibkrunConfig;
use uuid::Uuid;
use vm_trait::{NetworkMode, VmError, VmSpec};

pub(super) fn validate_runtime_git_bridge(
    config: &LibkrunConfig,
    spec: &VmSpec,
    authority: Option<&PreparedRuntimeAuthority>,
) -> Result<Option<PreparedRuntimeGitBridge>, VmError> {
    let Some(bridge) = spec.runtime_git_bridge else {
        return Ok(None);
    };
    if bridge.repository_id().is_nil() {
        return invalid("runtime_git_bridge.repository_id", "must not be nil");
    }
    if !(1024..=u16::MAX).contains(&bridge.loopback_port()) {
        return invalid(
            "runtime_git_bridge.loopback_port",
            "must be between 1024 and 65535",
        );
    }
    if !matches!(
        spec.network,
        NetworkMode::Disabled | NetworkMode::BrokerOnly
    ) {
        return invalid(
            "runtime_git_bridge.network",
            "requires disabled or broker-only networking",
        );
    }
    let authority = authority.ok_or_else(|| VmError::InvalidSpec {
        field: String::from("runtime_git_bridge"),
        reason: String::from("requires runtime authority"),
    })?;
    if authority.runtime_git_credential.is_none() {
        return invalid("runtime_git_bridge", "requires a runtime Git credential");
    }
    if spec.private_http_service.is_some() {
        return invalid(
            "runtime_git_bridge",
            "cannot combine with private HTTP service",
        );
    }
    if spec
        .labels
        .get("hephaestus.gateway.handler-contract")
        .is_some_and(|value| value == "http.v1")
    {
        return invalid(
            "runtime_git_bridge",
            "cannot combine with a gateway handler",
        );
    }
    if config.runtime_git_socket_path.is_none() {
        return invalid(
            "runtime_git_socket_path",
            "is required when a runtime Git bridge is declared",
        );
    }
    Ok(Some(PreparedRuntimeGitBridge {
        repository_id: bridge.repository_id(),
        loopback_port: bridge.loopback_port(),
    }))
}

pub(super) fn validate_private_http_service(
    spec: &VmSpec,
) -> Result<Option<PreparedPrivateHttpService>, VmError> {
    let Some(service) = spec.private_http_service.as_ref() else {
        return Ok(None);
    };

    if !(1024..=u16::MAX).contains(&service.loopback_port) {
        return invalid(
            "private_http_service.loopback_port",
            "must be between 1024 and 65535",
        );
    }
    if !(1..=64).contains(&service.max_connections) {
        return invalid(
            "private_http_service.max_connections",
            "must be between 1 and 64",
        );
    }
    let connect_timeout_ms =
        u64::try_from(service.connect_timeout.as_millis()).map_err(|_| VmError::InvalidSpec {
            field: "private_http_service.connect_timeout".to_owned(),
            reason: "must be representable as milliseconds".to_owned(),
        })?;
    if service.connect_timeout != std::time::Duration::from_millis(connect_timeout_ms)
        || !(1..=30_000).contains(&connect_timeout_ms)
    {
        return invalid(
            "private_http_service.connect_timeout",
            "must be a whole duration between 1ms and 30s",
        );
    }
    if spec.runtime_authority.is_some() {
        return invalid(
            "runtime_authority",
            "service mode cannot receive runtime authority",
        );
    }
    if spec
        .labels
        .get(crate::protocol::GATEWAY_HANDLER_CONTRACT_LABEL)
        .is_some_and(|value| value == crate::protocol::GATEWAY_HANDLER_CONTRACT_V1)
    {
        return invalid(
            "private_http_service",
            "service mode cannot combine with the stateless gateway handler",
        );
    }
    if !matches!(spec.network, NetworkMode::Disabled) {
        return invalid(
            "network",
            "initial service mode requires disabled guest networking",
        );
    }

    Ok(Some(PreparedPrivateHttpService {
        loopback_port: service.loopback_port,
        max_connections: service.max_connections,
        connect_timeout_ms,
    }))
}

pub(super) fn validate_state_volume_labels(
    spec: &VmSpec,
    disks: &[PreparedDisk],
) -> Result<(), VmError> {
    let filesystem_uuid = spec.labels.get("hephaestus.agent-state.filesystem-uuid");
    let mount_path = spec.labels.get("hephaestus.agent-state.mount-path");
    let scratch_uuid = spec.labels.get("hephaestus.oci-scratch.filesystem-uuid");
    let scratch_path = spec.labels.get("hephaestus.oci-scratch.mount-path");
    match (filesystem_uuid, mount_path, scratch_uuid, scratch_path) {
        (Some(_), Some(_), Some(_), Some(_)) => invalid(
            "labels",
            "agent-state and OCI scratch volumes are mutually exclusive",
        ),
        (None, None, None, None) => Ok(()),
        (Some(filesystem_uuid), Some(mount_path), None, None) => {
            if Uuid::parse_str(filesystem_uuid).is_err() {
                return invalid(
                    "labels.hephaestus.agent-state.filesystem-uuid",
                    "must be a valid UUID",
                );
            }
            if mount_path != "/var/lib/hephaestus" {
                return invalid(
                    "labels.hephaestus.agent-state.mount-path",
                    "must be /var/lib/hephaestus",
                );
            }
            if !disks
                .iter()
                .any(|disk| disk.id == "instance-state" && !disk.read_only)
            {
                return invalid(
                    "disks",
                    "state-volume labels require a writable instance-state disk",
                );
            }
            Ok(())
        }
        (None, None, Some(filesystem_uuid), Some(mount_path)) => {
            if Uuid::parse_str(filesystem_uuid).is_err() {
                return invalid(
                    "labels.hephaestus.oci-scratch.filesystem-uuid",
                    "must be a valid UUID",
                );
            }
            if mount_path != "/workspace/buildah" {
                return invalid(
                    "labels.hephaestus.oci-scratch.mount-path",
                    "must be /workspace/buildah",
                );
            }
            if !disks
                .iter()
                .any(|disk| disk.id == "repository-oci-scratch" && !disk.read_only)
            {
                return invalid(
                    "disks",
                    "OCI scratch labels require a writable repository-oci-scratch disk",
                );
            }
            Ok(())
        }
        _ => invalid(
            "labels",
            "volume filesystem UUID and mount path must be supplied together",
        ),
    }
}
