use crate::config::LibkrunConfig;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
    fs::{self, OpenOptions},
    net::{IpAddr, Ipv4Addr},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};
use uuid::Uuid;
use vm_trait::{DiskFormat, NetworkMode, PortProtocol, RootFilesystem, VmError, VmId, VmSpec};

pub const PROVIDER_NAME: &str = "libkrun";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreparedSpec {
    pub id: String,
    pub root: PreparedRoot,
    pub disks: Vec<PreparedDisk>,
    pub mounts: Vec<PreparedMount>,
    pub vcpus: u8,
    pub memory_mib: u32,
    pub network: PreparedNetwork,
    pub command: PreparedCommand,
    pub labels: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PreparedRoot {
    Directory { path: PathBuf },
    RawDisk { path: PathBuf, read_only: bool },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreparedDisk {
    pub id: String,
    pub path: PathBuf,
    pub read_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreparedMount {
    pub tag: String,
    pub host_path: PathBuf,
    pub guest_path: PathBuf,
    pub read_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PreparedNetwork {
    Disabled,
    UserMode { ingress: Vec<PreparedForward> },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct PreparedForward {
    pub bind_addr: IpAddr,
    pub host_port: u16,
    pub guest_port: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreparedCommand {
    pub program: String,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub working_dir: Option<PathBuf>,
}

pub fn validate_config(config: &LibkrunConfig) -> Result<(), VmError> {
    if config.service_uid == 0 {
        return invalid("service_uid", "the libkrun runtime must not run as root");
    }
    validate_service_identity(config)?;
    validate_absolute("runtime_root", &config.runtime_root)?;
    validate_directory("runtime_root", &config.runtime_root)?;
    validate_absolute("cgroup_root", &config.cgroup_root)?;
    validate_directory("cgroup_root", &config.cgroup_root)?;
    if config.enforce_cgroup_v2 && !config.cgroup_root.join("cgroup.controllers").is_file() {
        return invalid(
            "cgroup_root",
            "must be a delegated cgroup-v2 subtree containing cgroup.controllers",
        );
    }
    validate_executable("worker_binary", &config.worker_binary)?;
    validate_executable("passt_binary", &config.passt_binary)?;
    validate_allowed_roots("image_roots", &config.image_roots)?;
    validate_allowed_roots("disk_roots", &config.disk_roots)?;
    validate_allowed_roots("mount_roots", &config.mount_roots)?;

    OpenOptions::new()
        .read(true)
        .write(true)
        .open(&config.kvm_device)
        .map_err(|error| unavailable_error("KVM", error.to_string()))?;

    if config.limits.cpu_period_micros == 0 {
        return invalid("limits.cpu_period_micros", "must be greater than zero");
    }
    if config.limits.memory_max_bytes == 0 {
        return invalid("limits.memory_max_bytes", "must be greater than zero");
    }
    if config.limits.pids_max == 0 {
        return invalid("limits.pids_max", "must be greater than zero");
    }
    Ok(())
}

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
        command: PreparedCommand {
            program: spec.command.program.clone(),
            args: spec.command.args.clone(),
            env: spec.command.env.clone(),
            working_dir: spec.command.working_dir.clone(),
        },
        labels: spec.labels.clone(),
    })
}

fn validate_state_volume_labels(spec: &VmSpec, disks: &[PreparedDisk]) -> Result<(), VmError> {
    let filesystem_uuid = spec.labels.get("hephaestus.agent-state.filesystem-uuid");
    let mount_path = spec.labels.get("hephaestus.agent-state.mount-path");
    match (filesystem_uuid, mount_path) {
        (None, None) => Ok(()),
        (Some(filesystem_uuid), Some(mount_path)) => {
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
                .any(|disk| disk.id == "agent-state" && !disk.read_only)
            {
                return invalid(
                    "disks",
                    "state-volume labels require a writable agent-state disk",
                );
            }
            Ok(())
        }
        _ => invalid(
            "labels",
            "agent-state filesystem UUID and mount path must be supplied together",
        ),
    }
}

fn validate_service_identity(config: &LibkrunConfig) -> Result<(), VmError> {
    let uid = rustix::process::geteuid().as_raw();
    let gid = rustix::process::getegid().as_raw();
    if uid != config.service_uid || gid != config.service_gid {
        return unavailable(
            "service identity",
            format!(
                "effective identity {uid}:{gid} does not match configured {}:{}",
                config.service_uid, config.service_gid
            ),
        );
    }
    Ok(())
}

fn validate_allowed_roots(field: &str, roots: &[PathBuf]) -> Result<(), VmError> {
    if roots.is_empty() {
        return invalid(field, "must contain at least one allowed root");
    }
    for root in roots {
        validate_absolute(field, root)?;
        validate_directory(field, root)?;
    }
    Ok(())
}

fn validate_directory(field: &str, path: &Path) -> Result<(), VmError> {
    let metadata =
        fs::metadata(path).map_err(|error| unavailable_error(field, error.to_string()))?;
    if !metadata.is_dir() {
        return invalid(field, "must be a directory");
    }
    Ok(())
}

fn validate_executable(field: &str, path: &Path) -> Result<(), VmError> {
    validate_absolute(field, path)?;
    let metadata =
        fs::metadata(path).map_err(|error| unavailable_error(field, error.to_string()))?;
    if !metadata.is_file() || metadata.permissions().mode() & 0o111 == 0 {
        return invalid(field, "must be an executable regular file");
    }
    Ok(())
}

pub fn validate_id(id: &VmId) -> Result<(), VmError> {
    if id.0.is_empty()
        || id.0.len() > 64
        || !id
            .0
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return invalid(
            "id",
            "must be 1-64 ASCII letters, digits, hyphens, or underscores",
        );
    }
    Ok(())
}

fn canonical_allowed(
    field: &str,
    path: &Path,
    roots: &[PathBuf],
    kind: PathKind,
) -> Result<PathBuf, VmError> {
    validate_absolute(field, path)?;
    let canonical =
        fs::canonicalize(path).map_err(|error| invalid_error(field, error.to_string()))?;
    let allowed = roots.iter().any(|root| {
        fs::canonicalize(root).is_ok_and(|allowed_root| canonical.starts_with(allowed_root))
    });
    if !allowed {
        return invalid(field, "escapes configured allowed roots");
    }
    let metadata = fs::metadata(&canonical).map_err(provider_io)?;
    let valid_kind = match kind {
        PathKind::Directory => metadata.is_dir(),
        PathKind::File => metadata.is_file(),
    };
    if !valid_kind {
        return invalid(field, kind.description());
    }
    Ok(canonical)
}

fn validate_absolute(field: &str, path: &Path) -> Result<(), VmError> {
    if !path.is_absolute() {
        return invalid(field, "must be an absolute path");
    }
    Ok(())
}

fn validate_no_nul(field: &str, value: &str) -> Result<(), VmError> {
    if value.as_bytes().contains(&0) {
        return invalid(field, "must not contain NUL");
    }
    Ok(())
}

fn invalid<T>(field: impl Into<String>, reason: impl Into<String>) -> Result<T, VmError> {
    Err(invalid_error(field, reason))
}

fn invalid_error(field: impl Into<String>, reason: impl Into<String>) -> VmError {
    VmError::InvalidSpec {
        field: field.into(),
        reason: reason.into(),
    }
}

fn unsupported<T>(feature: impl Into<String>) -> Result<T, VmError> {
    Err(VmError::Unsupported {
        feature: feature.into(),
        provider: PROVIDER_NAME.to_owned(),
    })
}

fn unavailable<T>(resource: impl Into<String>, reason: impl Into<String>) -> Result<T, VmError> {
    Err(unavailable_error(resource, reason))
}

fn unavailable_error(resource: impl Into<String>, reason: impl Into<String>) -> VmError {
    VmError::Unavailable {
        resource: resource.into(),
        reason: reason.into(),
    }
}

fn provider_io(source: std::io::Error) -> VmError {
    VmError::Provider {
        provider: PROVIDER_NAME.to_owned(),
        code: "host-io".to_owned(),
        source: Box::new(source),
    }
}

#[derive(Clone, Copy)]
enum PathKind {
    Directory,
    File,
}

impl PathKind {
    const fn description(self) -> &'static str {
        match self {
            Self::Directory => "must be a directory",
            Self::File => "must be a regular file",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{prepare_spec, validate_config};
    use crate::config::LibkrunConfig;
    use std::{
        collections::BTreeMap,
        fs,
        net::{IpAddr, Ipv4Addr},
        os::unix::fs::{PermissionsExt, symlink},
        path::PathBuf,
    };
    use tempfile::TempDir;
    use vm_trait::{
        DiskFormat, GuestCommand, NetworkMode, PortForward, PortProtocol, RootFilesystem, VmDisk,
        VmError, VmId, VmMount, VmResources, VmSpec,
    };

    #[test]
    fn mount_escape_is_rejected() {
        let fixture = Fixture::new();
        let outside = fixture.temp.path().join("outside");
        fs::create_dir(&outside).unwrap();
        let mut spec = fixture.spec();
        spec.mounts.push(VmMount {
            tag: "repository".to_owned(),
            host_path: outside,
            guest_path: PathBuf::from("/repository"),
            read_only: true,
        });

        assert!(matches!(
            prepare_spec(&fixture.config, &spec),
            Err(VmError::InvalidSpec { field, .. }) if field == "mounts[0].host_path"
        ));
    }

    #[test]
    fn non_raw_root_is_typed_as_unsupported() {
        let fixture = Fixture::new();
        let image = fixture.images.join("root.qcow2");
        fs::write(&image, "image").unwrap();
        let mut spec = fixture.spec();
        spec.root = RootFilesystem::Disk {
            host_path: image,
            format: DiskFormat::Qcow2,
            read_only: true,
        };

        assert!(matches!(
            prepare_spec(&fixture.config, &spec),
            Err(VmError::Unsupported { .. })
        ));
    }

    #[test]
    fn forwarding_requires_tcp_on_ipv4_localhost() {
        let fixture = Fixture::new();
        let mut spec = fixture.spec();
        spec.network = NetworkMode::UserMode {
            ingress: vec![PortForward {
                protocol: PortProtocol::Tcp,
                bind_addr: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
                host_port: 0,
                guest_port: 22,
            }],
        };

        assert!(matches!(
            prepare_spec(&fixture.config, &spec),
            Err(VmError::InvalidSpec { field, .. })
                if field == "network.ingress[0].bind_addr"
        ));
    }

    #[test]
    fn valid_disk_and_mount_paths_are_preserved() {
        let fixture = Fixture::new();
        let disk = fixture.disks.join("sqlite.raw");
        fs::write(&disk, [0_u8; 16]).unwrap();
        let repository = fixture.mounts.join("repository");
        fs::create_dir(&repository).unwrap();
        let mut spec = fixture.spec();
        spec.disks.push(VmDisk {
            id: String::from("sqlite"),
            host_path: disk.clone(),
            format: DiskFormat::Raw,
            read_only: false,
        });
        spec.mounts.push(VmMount {
            tag: String::from("repository"),
            host_path: repository.clone(),
            guest_path: PathBuf::from("/repository"),
            read_only: true,
        });

        let prepared = prepare_spec(&fixture.config, &spec).unwrap();
        assert_eq!(prepared.disks[0].path, disk);
        assert_eq!(prepared.mounts[0].host_path, repository);
    }

    #[test]
    fn symlink_and_parent_traversal_escapes_are_rejected() {
        let fixture = Fixture::new();
        let outside = fixture.temp.path().join("outside.raw");
        fs::write(&outside, [0_u8; 1]).unwrap();
        let link = fixture.disks.join("escape.raw");
        symlink(&outside, &link).unwrap();
        let mut linked = fixture.spec();
        linked.disks.push(VmDisk {
            id: String::from("linked"),
            host_path: link,
            format: DiskFormat::Raw,
            read_only: true,
        });
        assert_invalid_field(prepare_spec(&fixture.config, &linked), "disks[0].host_path");

        let mut traversed = fixture.spec();
        traversed.disks.push(VmDisk {
            id: String::from("traversed"),
            host_path: fixture.disks.join("..").join("outside.raw"),
            format: DiskFormat::Raw,
            read_only: true,
        });
        assert_invalid_field(
            prepare_spec(&fixture.config, &traversed),
            "disks[0].host_path",
        );
    }

    #[test]
    fn missing_paths_and_wrong_file_types_are_rejected() {
        let fixture = Fixture::new();
        let mut missing = fixture.spec();
        missing.root = RootFilesystem::Directory {
            host_path: fixture.images.join("missing"),
        };
        assert_invalid_field(prepare_spec(&fixture.config, &missing), "root.host_path");

        let directory = fixture.disks.join("not-a-disk");
        fs::create_dir(&directory).unwrap();
        let mut wrong_disk = fixture.spec();
        wrong_disk.disks.push(VmDisk {
            id: String::from("wrong"),
            host_path: directory,
            format: DiskFormat::Raw,
            read_only: true,
        });
        assert_invalid_field(
            prepare_spec(&fixture.config, &wrong_disk),
            "disks[0].host_path",
        );

        let file = fixture.mounts.join("not-a-mount");
        fs::write(&file, []).unwrap();
        let mut wrong_mount = fixture.spec();
        wrong_mount.mounts.push(VmMount {
            tag: String::from("wrong"),
            host_path: file,
            guest_path: PathBuf::from("/wrong"),
            read_only: true,
        });
        assert_invalid_field(
            prepare_spec(&fixture.config, &wrong_mount),
            "mounts[0].host_path",
        );
    }

    #[test]
    fn duplicates_and_nul_arguments_are_rejected() {
        let fixture = Fixture::new();
        let mut argument = fixture.spec();
        argument.command.args.push(String::from("bad\0argument"));
        assert_invalid_field(prepare_spec(&fixture.config, &argument), "command.args[0]");

        let mut forwarding = fixture.spec();
        let duplicate = PortForward {
            protocol: PortProtocol::Tcp,
            bind_addr: IpAddr::V4(Ipv4Addr::LOCALHOST),
            host_port: 18080,
            guest_port: 80,
        };
        forwarding.network = NetworkMode::UserMode {
            ingress: vec![duplicate.clone(), duplicate],
        };
        assert_invalid_field(
            prepare_spec(&fixture.config, &forwarding),
            "network.ingress[1]",
        );
    }

    #[test]
    fn state_volume_labels_require_uuid_mount_and_writable_disk() {
        let fixture = Fixture::new();
        let mut missing_disk = fixture.spec();
        missing_disk.labels.insert(
            String::from("hephaestus.agent-state.filesystem-uuid"),
            uuid::Uuid::new_v4().to_string(),
        );
        missing_disk.labels.insert(
            String::from("hephaestus.agent-state.mount-path"),
            String::from("/var/lib/hephaestus"),
        );
        assert_invalid_field(
            prepare_spec(&fixture.valid_config(), &missing_disk),
            "disks",
        );

        let mut invalid_uuid = missing_disk;
        fs::write(fixture.disks.join("data.raw"), []).unwrap();
        invalid_uuid.disks.push(VmDisk {
            id: String::from("agent-state"),
            host_path: fixture.disks.join("data.raw"),
            format: DiskFormat::Raw,
            read_only: false,
        });
        invalid_uuid.labels.insert(
            String::from("hephaestus.agent-state.filesystem-uuid"),
            String::from("not-a-uuid"),
        );
        assert_invalid_field(
            prepare_spec(&fixture.valid_config(), &invalid_uuid),
            "labels.hephaestus.agent-state.filesystem-uuid",
        );
    }

    #[test]
    fn resource_and_writable_disk_limits_are_rejected() {
        let fixture = Fixture::new();
        let mut cpu = fixture.spec();
        cpu.resources.vcpus = 9;
        assert_invalid_field(prepare_spec(&fixture.config, &cpu), "resources.vcpus");

        let mut memory = fixture.spec();
        memory.resources.memory_mib = 4096;
        assert_invalid_field(
            prepare_spec(&fixture.config, &memory),
            "resources.memory_mib",
        );

        let disk = fixture.disks.join("oversized.raw");
        fs::write(&disk, [0_u8; 32]).unwrap();
        let mut config = fixture.config.clone();
        config.limits.writable_disk_max_bytes = 16;
        let mut storage = fixture.spec();
        storage.disks.push(VmDisk {
            id: String::from("oversized"),
            host_path: disk,
            format: DiskFormat::Raw,
            read_only: false,
        });
        assert_invalid_field(prepare_spec(&config, &storage), "disks");
    }

    #[test]
    fn host_preflight_returns_typed_configuration_errors() {
        let fixture = Fixture::new();
        let mut root_service = fixture.valid_config();
        root_service.service_uid = 0;
        assert!(matches!(
            validate_config(&root_service),
            Err(VmError::InvalidSpec { field, .. }) if field == "service_uid"
        ));

        let mut missing_kvm = fixture.valid_config();
        missing_kvm.kvm_device = fixture.temp.path().join("missing-kvm");
        assert!(matches!(
            validate_config(&missing_kvm),
            Err(VmError::Unavailable { resource, .. }) if resource == "KVM"
        ));

        let non_executable = fixture.temp.path().join("not-executable");
        fs::write(&non_executable, []).unwrap();
        fs::set_permissions(&non_executable, fs::Permissions::from_mode(0o600)).unwrap();
        let mut invalid_passt = fixture.valid_config();
        invalid_passt.passt_binary = non_executable;
        assert!(matches!(
            validate_config(&invalid_passt),
            Err(VmError::InvalidSpec { field, .. }) if field == "passt_binary"
        ));

        let mut missing_delegation = fixture.valid_config();
        missing_delegation.enforce_cgroup_v2 = true;
        assert!(matches!(
            validate_config(&missing_delegation),
            Err(VmError::InvalidSpec { field, .. }) if field == "cgroup_root"
        ));
    }

    #[test]
    fn host_preflight_rejects_relative_roots_and_zero_limits() {
        let fixture = Fixture::new();
        let mut relative = fixture.valid_config();
        relative.image_roots = vec![PathBuf::from("images")];
        assert!(matches!(
            validate_config(&relative),
            Err(VmError::InvalidSpec { field, .. }) if field == "image_roots"
        ));

        let mut zero_period = fixture.valid_config();
        zero_period.limits.cpu_period_micros = 0;
        assert!(matches!(
            validate_config(&zero_period),
            Err(VmError::InvalidSpec { field, .. }) if field == "limits.cpu_period_micros"
        ));

        let mut zero_memory = fixture.valid_config();
        zero_memory.limits.memory_max_bytes = 0;
        assert!(matches!(
            validate_config(&zero_memory),
            Err(VmError::InvalidSpec { field, .. }) if field == "limits.memory_max_bytes"
        ));

        let mut zero_pids = fixture.valid_config();
        zero_pids.limits.pids_max = 0;
        assert!(matches!(
            validate_config(&zero_pids),
            Err(VmError::InvalidSpec { field, .. }) if field == "limits.pids_max"
        ));
    }

    fn assert_invalid_field(result: Result<super::PreparedSpec, VmError>, expected: &str) {
        assert!(matches!(
            result,
            Err(VmError::InvalidSpec { field, .. }) if field.starts_with(expected)
        ));
    }

    struct Fixture {
        temp: TempDir,
        images: PathBuf,
        disks: PathBuf,
        mounts: PathBuf,
        config: LibkrunConfig,
    }

    impl Fixture {
        fn new() -> Self {
            let temp = TempDir::new().unwrap();
            let images = temp.path().join("images");
            let disks = temp.path().join("disks");
            let mounts = temp.path().join("mounts");
            for directory in [&images, &disks, &mounts] {
                fs::create_dir(directory).unwrap();
            }
            let root = images.join("root");
            fs::create_dir(&root).unwrap();
            let config = LibkrunConfig::new(
                temp.path(),
                vec![images.clone()],
                vec![disks.clone()],
                vec![mounts.clone()],
                "/bin/true",
                temp.path(),
            );
            Self {
                temp,
                images,
                disks,
                mounts,
                config,
            }
        }

        fn spec(&self) -> VmSpec {
            VmSpec {
                id: VmId("validation".to_owned()),
                root: RootFilesystem::Directory {
                    host_path: self.images.join("root"),
                },
                disks: Vec::new(),
                mounts: Vec::new(),
                resources: VmResources {
                    vcpus: 1,
                    memory_mib: 256,
                },
                network: NetworkMode::Disabled,
                command: GuestCommand {
                    program: "/bin/true".to_owned(),
                    args: Vec::new(),
                    env: BTreeMap::new(),
                    working_dir: Some(PathBuf::from("/")),
                },
                labels: BTreeMap::new(),
            }
        }

        fn valid_config(&self) -> LibkrunConfig {
            let kvm = self.temp.path().join("kvm");
            fs::write(&kvm, []).unwrap();
            let mut config = self.config.clone();
            config.passt_binary = PathBuf::from("/bin/true");
            config.kvm_device = kvm;
            config.enforce_cgroup_v2 = false;
            config
        }
    }
}
