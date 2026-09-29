use super::helpers::{invalid, unavailable, unavailable_error, validate_absolute};
use crate::config::LibkrunConfig;
use std::{
    fs::{self, OpenOptions},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};
use vm_trait::VmError;

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
    if let Some(path) = &config.broker_socket_path {
        validate_absolute("broker_socket_path", path)?;
    }
    if let Some(path) = &config.runtime_git_socket_path {
        validate_absolute("runtime_git_socket_path", path)?;
    }
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

pub(super) fn validate_service_identity(config: &LibkrunConfig) -> Result<(), VmError> {
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

pub(super) fn validate_allowed_roots(field: &str, roots: &[PathBuf]) -> Result<(), VmError> {
    if roots.is_empty() {
        return invalid(field, "must contain at least one allowed root");
    }
    for root in roots {
        validate_absolute(field, root)?;
        validate_directory(field, root)?;
    }
    Ok(())
}

pub(super) fn validate_directory(field: &str, path: &Path) -> Result<(), VmError> {
    let metadata =
        fs::metadata(path).map_err(|error| unavailable_error(field, error.to_string()))?;
    if !metadata.is_dir() {
        return invalid(field, "must be a directory");
    }
    Ok(())
}

pub(super) fn validate_executable(field: &str, path: &Path) -> Result<(), VmError> {
    validate_absolute(field, path)?;
    let metadata =
        fs::metadata(path).map_err(|error| unavailable_error(field, error.to_string()))?;
    if !metadata.is_file() || metadata.permissions().mode() & 0o111 == 0 {
        return invalid(field, "must be an executable regular file");
    }
    Ok(())
}
