use super::MAX_VIRTIO_FS_TAG_BYTES;
use super::{
    helpers::{
        PathKind, canonical_allowed, invalid, provider_io, unsupported, validate_absolute,
        validate_id, validate_no_nul,
    },
    policies::{
        validate_private_http_service, validate_runtime_git_bridge, validate_state_volume_labels,
    },
    types::{
        PreparedCommand, PreparedDisk, PreparedForward, PreparedMount, PreparedNetwork,
        PreparedRoot, PreparedRuntimeAuthority, PreparedSpec,
    },
};
use crate::config::LibkrunConfig;
use std::{
    collections::HashSet,
    fs,
    net::{IpAddr, Ipv4Addr},
    path::Path,
};
use vm_trait::{DiskFormat, NetworkMode, PortProtocol, RootFilesystem, VmError, VmSpec};

// Keeping the validation pipeline together makes its cleanup and security
// invariants auditable from one entry point.
#[allow(clippy::too_many_lines)]
pub fn prepare_spec(config: &LibkrunConfig, spec: &VmSpec) -> Result<PreparedSpec, VmError> {
    validate_id(&spec.id)?;
    if spec.resources.vcpus == 0 || spec.resources.vcpus > 8 {
        return invalid("resources.vcpus", "must be between 1 and 8");
    }
    if spec.resources.memory_mib == 0 {
        return invalid("resources.memory_mib", "must be greater than zero");
    }
    let requested_memory = u64::from(spec.resources.memory_mib) * 1024 * 1024;
    if requested_memory > config.limits.memory_max_bytes {
        return invalid(
            "resources.memory_mib",
            "exceeds the configured cgroup memory limit",
        );
    }

    validate_absolute("command.program", Path::new(&spec.command.program))?;
    validate_no_nul("command.program", &spec.command.program)?;
    if let Some(path) = &spec.command.working_dir {
        validate_absolute("command.working_dir", path)?;
    }
    for (index, argument) in spec.command.args.iter().enumerate() {
        validate_no_nul(&format!("command.args[{index}]"), argument)?;
    }
    for (key, value) in &spec.command.env {
        validate_no_nul("command.env key", key)?;
        validate_no_nul("command.env value", value)?;
        if key.contains('=') {
            return invalid("command.env", "keys must not contain '='");
        }
    }

    let root = match &spec.root {
        RootFilesystem::Directory { host_path } => PreparedRoot::Directory {
            path: canonical_allowed(
                "root.host_path",
                host_path,
                &config.image_roots,
                PathKind::Directory,
            )?,
        },
        RootFilesystem::Disk {
            host_path,
            format: DiskFormat::Raw,
            read_only,
        } => PreparedRoot::RawDisk {
            path: canonical_allowed(
                "root.host_path",
                host_path,
                &config.image_roots,
                PathKind::File,
            )?,
            read_only: *read_only,
        },
        RootFilesystem::Disk { format, .. } => {
            return unsupported(format!("root disk format {format:?}"));
        }
        _ => return unsupported("root filesystem"),
    };
    let mut disk_ids = HashSet::new();
    let mut writable_bytes = 0_u64;
    let mut disks = Vec::with_capacity(spec.disks.len());
    for (index, disk) in spec.disks.iter().enumerate() {
        if disk.id.is_empty() || !disk_ids.insert(disk.id.as_str()) {
            return invalid(format!("disks[{index}].id"), "must be non-empty and unique");
        }
        validate_no_nul(&format!("disks[{index}].id"), &disk.id)?;
        if !matches!(disk.format, DiskFormat::Raw) {
            return unsupported(format!("disk format {:?}", disk.format));
        }
        let path = canonical_allowed(
            &format!("disks[{index}].host_path"),
            &disk.host_path,
            &config.disk_roots,
            PathKind::File,
        )?;
        if !disk.read_only {
            writable_bytes = writable_bytes
                .checked_add(fs::metadata(&path).map_err(provider_io)?.len())
                .ok_or_else(|| VmError::InvalidSpec {
                    field: "disks".to_owned(),
                    reason: "aggregate writable disk size overflowed".to_owned(),
                })?;
        }
        disks.push(PreparedDisk {
            id: disk.id.clone(),
            path,
            read_only: disk.read_only,
        });
        tracing::debug!(
            disk_id = %disk.id,
            disk_path = %disk.host_path.display(),
            read_only = disk.read_only,
            "validated VM disk"
        );
    }
    if writable_bytes > config.limits.writable_disk_max_bytes {
        return invalid(
            "disks",
            "aggregate writable disk size exceeds configured limit",
        );
    }
    validate_state_volume_labels(spec, &disks)?;

    let runtime_authority = spec
        .runtime_authority
        .as_ref()
        .map(|authority| {
            if authority.generation() == 0 {
                return invalid("runtime_authority.generation", "must be greater than zero");
            }
            Ok(PreparedRuntimeAuthority {
                session_id: authority.session_id(),
                generation: authority.generation(),
                credential: *authority.credential(),
                runtime_git_credential: authority.runtime_git_credential().copied(),
            })
        })
        .transpose()?;

    let private_http_service = validate_private_http_service(spec)?;
    let runtime_git_bridge = validate_runtime_git_bridge(config, spec, runtime_authority.as_ref())?;

    let mut mount_tags = HashSet::new();
    let mut mounts = Vec::with_capacity(spec.mounts.len());
    for (index, mount) in spec.mounts.iter().enumerate() {
        if mount.tag.is_empty() || !mount_tags.insert(mount.tag.as_str()) {
            return invalid(
                format!("mounts[{index}].tag"),
                "must be non-empty and unique",
            );
        }
        validate_no_nul(&format!("mounts[{index}].tag"), &mount.tag)?;
        if mount.tag.len() > MAX_VIRTIO_FS_TAG_BYTES {
            return invalid(
                format!("mounts[{index}].tag"),
                "exceeds libkrun's 36-byte virtio-fs tag limit",
            );
        }
        validate_absolute(&format!("mounts[{index}].guest_path"), &mount.guest_path)?;
        let host_path = canonical_allowed(
            &format!("mounts[{index}].host_path"),
            &mount.host_path,
            &config.mount_roots,
            PathKind::Directory,
        )?;
        mounts.push(PreparedMount {
            tag: mount.tag.clone(),
            host_path,
            guest_path: mount.guest_path.clone(),
            read_only: mount.read_only,
        });
        tracing::debug!(
            mount_tag = %mount.tag,
            mount_path = %mount.host_path.display(),
            guest_path = %mount.guest_path.display(),
            read_only = mount.read_only,
            "validated virtio-fs mount"
        );
    }

    let network = match &spec.network {
        NetworkMode::Disabled => PreparedNetwork::Disabled,
        NetworkMode::BrokerOnly => {
            if config.broker_socket_path.is_none() {
                return invalid(
                    "network",
                    "broker-only mode requires a configured broker socket",
                );
            }
            PreparedNetwork::BrokerOnly
        }
        NetworkMode::UserMode { ingress } => {
            let mut forwards = Vec::with_capacity(ingress.len());
            let mut fixed_bindings = HashSet::new();
            for (index, forward) in ingress.iter().enumerate() {
                if !matches!(forward.protocol, PortProtocol::Tcp) {
                    return unsupported("non-TCP ingress forwarding");
                }
                if forward.bind_addr != IpAddr::V4(Ipv4Addr::LOCALHOST) {
                    return invalid(
                        format!("network.ingress[{index}].bind_addr"),
                        "libkrun ingress must bind to 127.0.0.1",
                    );
                }
                if forward.guest_port == 0 {
                    return invalid(
                        format!("network.ingress[{index}].guest_port"),
                        "must be greater than zero",
                    );
                }
                if forward.host_port != 0
                    && !fixed_bindings.insert((
                        forward.protocol,
                        forward.bind_addr,
                        forward.host_port,
                    ))
                {
                    return invalid(
                        format!("network.ingress[{index}]"),
                        "duplicates an earlier fixed host binding",
                    );
                }
                forwards.push(PreparedForward {
                    bind_addr: forward.bind_addr,
                    host_port: forward.host_port,
                    guest_port: forward.guest_port,
                });
            }
            PreparedNetwork::UserMode { ingress: forwards }
        }
        _ => return unsupported("network mode"),
    };

    Ok(PreparedSpec {
        id: spec.id.0.clone(),
        root,
        disks,
        mounts,
        vcpus: spec.resources.vcpus,
        memory_mib: spec.resources.memory_mib,
        network,
        private_http_service,
        runtime_git_bridge,
        command: PreparedCommand {
            program: spec.command.program.clone(),
            args: spec.command.args.clone(),
            env: spec.command.env.clone(),
            working_dir: spec.command.working_dir.clone(),
        },
        runtime_authority,
        labels: spec.labels.clone(),
    })
}
