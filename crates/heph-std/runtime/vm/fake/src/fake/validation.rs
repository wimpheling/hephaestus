use std::{collections::HashSet, path::Path};
use vm_trait::{NetworkMode, PortProtocol, RootFilesystem, VmError, VmSpec};

// Keeping fake validation aligned with the public contract is clearer than
// splitting its small, sequential validation pipeline.
#[allow(clippy::too_many_lines)]
pub fn validate_spec(spec: &VmSpec) -> Result<(), VmError> {
    validate_nonempty("id", &spec.id.0)?;
    if spec.resources.vcpus == 0 {
        return invalid_spec("resources.vcpus", "must be greater than zero");
    }
    if spec.resources.memory_mib == 0 {
        return invalid_spec("resources.memory_mib", "must be greater than zero");
    }
    validate_absolute("command.program", Path::new(&spec.command.program))?;
    validate_no_nul("command.program", &spec.command.program)?;
    if let Some(working_dir) = &spec.command.working_dir {
        validate_absolute("command.working_dir", working_dir)?;
    }
    for (index, argument) in spec.command.args.iter().enumerate() {
        validate_no_nul(&format!("command.args[{index}]"), argument)?;
    }
    for (key, value) in &spec.command.env {
        validate_no_nul("command.env", key)?;
        validate_no_nul("command.env", value)?;
        if key.contains('=') {
            return invalid_spec("command.env", "keys must not contain '='");
        }
    }
    if spec
        .runtime_authority
        .as_ref()
        .is_some_and(|authority| authority.generation() == 0)
    {
        return invalid_spec("runtime_authority.generation", "must be greater than zero");
    }
    if let Some(service) = &spec.private_http_service {
        if !(1024..=u16::MAX).contains(&service.loopback_port) {
            return invalid_spec(
                "private_http_service.loopback_port",
                "must be between 1024 and 65535",
            );
        }
        if !(1..=64).contains(&service.max_connections) {
            return invalid_spec(
                "private_http_service.max_connections",
                "must be between 1 and 64",
            );
        }
        let connect_timeout_ms =
            u64::try_from(service.connect_timeout.as_millis()).map_err(|_| {
                VmError::InvalidSpec {
                    field: "private_http_service.connect_timeout".to_owned(),
                    reason: "must be representable as milliseconds".to_owned(),
                }
            })?;
        if service.connect_timeout != std::time::Duration::from_millis(connect_timeout_ms)
            || !(1..=30_000).contains(&connect_timeout_ms)
        {
            return invalid_spec(
                "private_http_service.connect_timeout",
                "must be a whole duration between 1ms and 30s",
            );
        }
        if spec.runtime_authority.is_some() {
            return invalid_spec(
                "runtime_authority",
                "service mode cannot receive runtime authority",
            );
        }
        if spec
            .labels
            .get("hephaestus.gateway.handler-contract")
            .is_some_and(|value| value == "http.v1")
        {
            return invalid_spec(
                "private_http_service",
                "service mode cannot combine with the stateless gateway handler",
            );
        }
        if !matches!(spec.network, NetworkMode::Disabled) {
            return invalid_spec(
                "network",
                "initial service mode requires disabled guest networking",
            );
        }
    }

    match &spec.root {
        RootFilesystem::Directory { host_path } | RootFilesystem::Disk { host_path, .. } => {
            validate_absolute("root.host_path", host_path)?;
        }
        _ => {
            return Err(VmError::Unsupported {
                feature: "root filesystem".to_owned(),
                provider: "fake".to_owned(),
            });
        }
    }

    let mut disk_ids = HashSet::new();
    for (index, disk) in spec.disks.iter().enumerate() {
        validate_nonempty(&format!("disks[{index}].id"), &disk.id)?;
        validate_absolute(&format!("disks[{index}].host_path"), &disk.host_path)?;
        if !disk_ids.insert(&disk.id) {
            return invalid_spec(
                &format!("disks[{index}].id"),
                "must be unique within the VM specification",
            );
        }
    }

    let mut mount_tags = HashSet::new();
    for (index, mount) in spec.mounts.iter().enumerate() {
        validate_nonempty(&format!("mounts[{index}].tag"), &mount.tag)?;
        validate_absolute(&format!("mounts[{index}].host_path"), &mount.host_path)?;
        validate_absolute(&format!("mounts[{index}].guest_path"), &mount.guest_path)?;
        if !mount_tags.insert(&mount.tag) {
            return invalid_spec(
                &format!("mounts[{index}].tag"),
                "must be unique within the VM specification",
            );
        }
    }

    match &spec.network {
        NetworkMode::Disabled => {}
        NetworkMode::UserMode { ingress } => {
            let mut fixed_bindings = HashSet::new();
            for (index, forward) in ingress.iter().enumerate() {
                if !forward.bind_addr.is_loopback() {
                    return invalid_spec(
                        &format!("network.ingress[{index}].bind_addr"),
                        "must be a loopback address",
                    );
                }
                if forward.guest_port == 0 {
                    return invalid_spec(
                        &format!("network.ingress[{index}].guest_port"),
                        "must be greater than zero",
                    );
                }
                if !matches!(forward.protocol, PortProtocol::Tcp) {
                    return Err(VmError::Unsupported {
                        feature: "port-forward protocol".to_owned(),
                        provider: "fake".to_owned(),
                    });
                }
                if forward.host_port != 0
                    && !fixed_bindings.insert((
                        forward.protocol,
                        forward.bind_addr,
                        forward.host_port,
                    ))
                {
                    return invalid_spec(
                        &format!("network.ingress[{index}]"),
                        "duplicates an earlier fixed host binding",
                    );
                }
            }
        }
        _ => {
            return Err(VmError::Unsupported {
                feature: "network mode".to_owned(),
                provider: "fake".to_owned(),
            });
        }
    }

    Ok(())
}

fn validate_nonempty(field: &str, value: &str) -> Result<(), VmError> {
    if value.is_empty() {
        return invalid_spec(field, "must not be empty");
    }
    Ok(())
}

fn validate_absolute(field: &str, path: &Path) -> Result<(), VmError> {
    if !path.is_absolute() {
        return invalid_spec(field, "must be an absolute path");
    }
    Ok(())
}

fn validate_no_nul(field: &str, value: &str) -> Result<(), VmError> {
    if value.as_bytes().contains(&0) {
        return invalid_spec(field, "must not contain NUL");
    }
    Ok(())
}

fn invalid_spec<T>(field: &str, reason: &str) -> Result<T, VmError> {
    Err(VmError::InvalidSpec {
        field: field.to_owned(),
        reason: reason.to_owned(),
    })
}
