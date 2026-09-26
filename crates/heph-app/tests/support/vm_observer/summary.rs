use super::{
    AGENT_PATHS, BUILD_PATHS, CommandClass, CommandSummary, DiskSummary, GATEWAY_PATHS,
    MountSummary, NetworkSummary, VmSpecSummary,
};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use vm_trait::{NetworkMode, RootFilesystem, VmError, VmId, VmSpec};

pub fn summarize_spec(spec: &VmSpec, patterns: &[Vec<u8>]) -> Result<VmSpecSummary, VmError> {
    let vm_id = safe_vm_id(&spec.id, patterns)?;
    scan_value(spec.command.program.as_bytes(), patterns, "command")?;
    for argument in &spec.command.args {
        scan_value(argument.as_bytes(), patterns, "command argument")?;
    }
    for value in spec.command.env.values() {
        scan_value(value.as_bytes(), patterns, "command environment")?;
    }
    for key in spec.command.env.keys() {
        scan_value(key.as_bytes(), patterns, "command environment key")?;
    }

    let root_exists = match &spec.root {
        RootFilesystem::Directory { host_path } => host_path.is_dir(),
        RootFilesystem::Disk { host_path, .. } => host_path.is_file(),
        _ => false,
    };
    if !root_exists {
        return Err(observer_error("VM root backing resource is unavailable"));
    }
    let disks = spec
        .disks
        .iter()
        .map(|disk| DiskSummary {
            id: safe_disk_id(&disk.id),
            read_only: disk.read_only,
            host_exists: disk.host_path.is_file(),
        })
        .collect::<Vec<_>>();
    if disks.iter().any(|disk| !disk.host_exists) {
        return Err(observer_error("VM disk backing resource is unavailable"));
    }
    let mounts = spec
        .mounts
        .iter()
        .map(|mount| MountSummary {
            tag_fingerprint: fingerprint(mount.tag.as_bytes()),
            guest_path: safe_guest_path(&mount.guest_path),
            read_only: mount.read_only,
            host_exists: mount.host_path.is_dir(),
        })
        .collect::<Vec<_>>();
    if mounts.iter().any(|mount| !mount.host_exists) {
        return Err(observer_error("VM mount backing resource is unavailable"));
    }
    let mut environment_keys = spec.command.env.keys().cloned().collect::<Vec<_>>();
    environment_keys.sort();
    Ok(VmSpecSummary {
        vm_id,
        kind: safe_kind(spec),
        root_exists,
        disks,
        mounts,
        network: match &spec.network {
            NetworkMode::Disabled => NetworkSummary::Disabled,
            NetworkMode::BrokerOnly => NetworkSummary::BrokerOnly,
            NetworkMode::UserMode { ingress } => NetworkSummary::UserMode {
                ingress_count: ingress.len(),
            },
            _ => NetworkSummary::Unknown,
        },
        command: CommandSummary {
            class: command_class(&spec.command.program),
            argument_fingerprints: spec
                .command
                .args
                .iter()
                .map(|argument| fingerprint(argument.as_bytes()))
                .collect(),
            environment_keys,
        },
        runtime_authority: spec.runtime_authority.is_some(),
    })
}

fn safe_vm_id(id: &VmId, patterns: &[Vec<u8>]) -> Result<String, VmError> {
    scan_value(id.0.as_bytes(), patterns, "VM identifier")?;
    let is_uuid = Uuid::parse_str(&id.0).is_ok();
    let is_prefixed_uuid = ["agent-", "gateway-", "build-"].iter().any(|prefix| {
        id.0.strip_prefix(prefix)
            .is_some_and(|suffix| Uuid::parse_str(suffix).is_ok())
    });
    if is_uuid || is_prefixed_uuid {
        Ok(id.0.clone())
    } else {
        Ok(fingerprint(id.0.as_bytes()))
    }
}

fn scan_value(value: &[u8], patterns: &[Vec<u8>], field: &'static str) -> Result<(), VmError> {
    if patterns.iter().any(|pattern| {
        !pattern.is_empty() && value.windows(pattern.len()).any(|window| window == pattern)
    }) {
        return Err(observer_error(field));
    }
    Ok(())
}

pub fn observer_error(reason: &'static str) -> VmError {
    VmError::InvalidSpec {
        field: String::from("vm_observer"),
        reason: reason.to_owned(),
    }
}

fn safe_kind(spec: &VmSpec) -> String {
    match spec.labels.get("hephaestus.kind").map(String::as_str) {
        Some("gateway") => String::from("gateway"),
        Some("build") => String::from("build"),
        Some("repository_oci_builder") => String::from("repository_oci_builder"),
        Some("repository_oci_verifier") => String::from("repository_oci_verifier"),
        Some("agent") => String::from("agent"),
        None if spec.labels.contains_key("hephaestus.instance") => String::from("agent"),
        Some(_) | None => String::from("other"),
    }
}

fn safe_disk_id(id: &str) -> String {
    if id == "instance-state" {
        String::from(id)
    } else {
        fingerprint(id.as_bytes())
    }
}

fn fingerprint(value: &[u8]) -> String {
    let mut digest = Sha256::new();
    digest.update(value);
    format!("sha256:{:x}", digest.finalize())
}

fn safe_guest_path(path: &std::path::Path) -> String {
    let value = path.to_string_lossy();
    if AGENT_PATHS
        .iter()
        .chain(GATEWAY_PATHS.iter())
        .chain(BUILD_PATHS.iter())
        .any(|known| *known == value)
        || value == "/release-previous"
    {
        value.into_owned()
    } else {
        fingerprint(value.as_bytes())
    }
}

fn command_class(program: &str) -> CommandClass {
    if program.starts_with("/release/") {
        CommandClass::Release
    } else if program == "/bin/sh" || program == "/usr/bin/env" {
        CommandClass::Build
    } else {
        CommandClass::Other
    }
}

pub fn has_exact_paths(mounts: &[MountSummary], expected: &[&str]) -> bool {
    let paths: Vec<_> = mounts
        .iter()
        .map(|mount| mount.guest_path.as_str())
        .collect();
    expected.iter().all(|path| paths.contains(path)) && paths.len() == expected.len()
}

pub fn mount_is_read_only(mounts: &[MountSummary], path: &str, expected: bool) -> bool {
    mounts
        .iter()
        .find(|mount| mount.guest_path == path)
        .is_some_and(|mount| mount.read_only == expected)
}

pub fn all_agent_mounts_are_read_only_except_work(mounts: &[MountSummary]) -> bool {
    mounts.iter().all(|mount| {
        if mount.guest_path == "/workspace/work" {
            !mount.read_only
        } else {
            mount.read_only
        }
    })
}

#[cfg(test)]
#[path = "summary_tests.rs"]
mod tests;
