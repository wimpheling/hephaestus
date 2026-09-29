use run_orchestrator::{RunRuntimeCatalogError, RunRuntimeError};
use runtime_types::RunId;
use std::{
    fs::{self, OpenOptions},
    io::Read,
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::Path,
};
use uuid::Uuid;

use crate::{
    GATEWAY_RELEASE_TAG_PREFIX, MAX_GATEWAY_SERVICE_METADATA_BYTES,
    types::{GatewayServiceIdentity, GatewayServiceIdentityFile},
};

pub fn ensure_existing_directory(path: &Path) -> Result<(), RunRuntimeError> {
    let metadata = fs::symlink_metadata(path).map_err(filesystem)?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err(runtime_error("runtime directory is unsafe"));
    }
    Ok(())
}

pub fn read_service_identity(path: &Path) -> Result<GatewayServiceIdentity, RunRuntimeError> {
    let initial = fs::symlink_metadata(path).map_err(filesystem)?;
    validate_service_identity_metadata(&initial)?;
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(o_nofollow() | o_nonblock())
        .open(path)
        .map_err(filesystem)?;
    let metadata = file.metadata().map_err(filesystem)?;
    validate_service_identity_metadata(&metadata)?;
    let limit =
        usize::try_from(MAX_GATEWAY_SERVICE_METADATA_BYTES).expect("metadata limit fits in usize");
    let mut bytes = Vec::with_capacity(limit);
    file.take(MAX_GATEWAY_SERVICE_METADATA_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(filesystem)?;
    if bytes.len() > limit {
        return Err(runtime_error(
            "gateway service identity metadata is too large",
        ));
    }
    let metadata: GatewayServiceIdentityFile = serde_json::from_slice(&bytes)
        .map_err(|_| runtime_error("gateway service identity metadata is invalid"))?;
    metadata.try_into()
}

pub fn validate_service_identity_metadata(
    metadata: &std::fs::Metadata,
) -> Result<(), RunRuntimeError> {
    if !metadata.file_type().is_file()
        || metadata.nlink() != 1
        || metadata.permissions().mode() & 0o222 != 0
        || metadata.len() > MAX_GATEWAY_SERVICE_METADATA_BYTES
    {
        return Err(runtime_error("gateway service identity metadata is unsafe"));
    }
    Ok(())
}

pub fn validate_sealed_service_subtree(path: &Path) -> Result<(), RunRuntimeError> {
    let metadata = fs::symlink_metadata(path).map_err(filesystem)?;
    if !metadata.file_type().is_dir()
        || metadata.file_type().is_symlink()
        || metadata.permissions().mode() & 0o222 != 0
    {
        return Err(runtime_error("gateway service mount tree is unsafe"));
    }
    for entry in fs::read_dir(path).map_err(filesystem)? {
        let entry = entry.map_err(filesystem)?;
        let metadata = fs::symlink_metadata(entry.path()).map_err(filesystem)?;
        if metadata.file_type().is_dir() {
            if metadata.file_type().is_symlink() {
                return Err(runtime_error("gateway service mount tree is unsafe"));
            }
            validate_sealed_service_subtree(&entry.path())?;
        } else if !metadata.file_type().is_file() || metadata.permissions().mode() & 0o222 != 0 {
            return Err(runtime_error("gateway service mount tree is unsafe"));
        }
    }
    Ok(())
}

pub fn make_tree_read_only(root: &Path) -> Result<(), RunRuntimeError> {
    for entry in fs::read_dir(root).map_err(filesystem)? {
        let entry = entry.map_err(filesystem)?;
        let metadata = entry.metadata().map_err(filesystem)?;
        if metadata.file_type().is_dir() {
            make_tree_read_only(&entry.path())?;
            fs::set_permissions(entry.path(), fs::Permissions::from_mode(0o555))
                .map_err(filesystem)?;
        }
    }
    fs::set_permissions(root, fs::Permissions::from_mode(0o555)).map_err(filesystem)
}

pub fn make_tree_removable(root: &Path) -> Result<(), RunRuntimeError> {
    let metadata = fs::symlink_metadata(root).map_err(filesystem)?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err(runtime_error("run runtime tree is unsafe"));
    }
    fs::set_permissions(root, fs::Permissions::from_mode(0o700)).map_err(filesystem)?;
    for entry in fs::read_dir(root).map_err(filesystem)? {
        let entry = entry.map_err(filesystem)?;
        let metadata = fs::symlink_metadata(entry.path()).map_err(filesystem)?;
        if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() {
            make_tree_removable(&entry.path())?;
        } else if !metadata.file_type().is_file() {
            return Err(runtime_error("run runtime tree is unsafe"));
        }
    }
    Ok(())
}

pub fn create_directory(path: &Path, mode: u32) -> Result<(), RunRuntimeError> {
    fs::create_dir(path).map_err(filesystem)?;
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).map_err(filesystem)
}

pub fn validate_root(path: &Path) -> Result<(), RunRuntimeError> {
    let metadata = fs::symlink_metadata(path).map_err(filesystem)?;
    if !metadata.file_type().is_dir()
        || metadata.file_type().is_symlink()
        || metadata.permissions().mode() & 0o022 != 0
    {
        return Err(runtime_error("runtime root is unsafe"));
    }
    Ok(())
}

pub fn runtime_mount_tag(prefix: &str, run_id: RunId) -> String {
    format!("{prefix}-{}", run_id.as_uuid().simple())
}

pub fn gateway_runtime_mount_tag(invocation_id: Uuid) -> String {
    format!("{GATEWAY_RELEASE_TAG_PREFIX}-{}", invocation_id.simple())
}

pub fn gateway_service_mount_tag(prefix: &str, instance_id: Uuid) -> String {
    format!("{prefix}-{}", instance_id.simple())
}

#[cfg(target_os = "linux")]
pub const fn o_nofollow() -> i32 {
    0o400_000 | 0o2_000_000
}

#[cfg(not(target_os = "linux"))]
pub const fn o_nofollow() -> i32 {
    0
}

#[cfg(target_os = "linux")]
pub const fn o_nonblock() -> i32 {
    0o000_4000
}

#[cfg(not(target_os = "linux"))]
pub const fn o_nonblock() -> i32 {
    0
}

pub fn runtime_error(message: impl Into<String>) -> RunRuntimeError {
    RunRuntimeError::redacted(message)
}

// Error details may contain host paths or catalog diagnostics, so only stable classes
// cross the runtime-manager boundary.
#[allow(clippy::needless_pass_by_value)]
pub fn filesystem(error: std::io::Error) -> RunRuntimeError {
    tracing::warn!(
        error_kind = ?error.kind(),
        raw_os_error = ?error.raw_os_error(),
        "run runtime filesystem operation failed"
    );
    runtime_error("runtime filesystem operation failed")
}

// See `filesystem`: catalog diagnostics are intentionally redacted.
#[allow(clippy::needless_pass_by_value)]
pub fn catalog(_error: RunRuntimeCatalogError) -> RunRuntimeError {
    runtime_error("runtime provenance query failed")
}

// See `filesystem`: serialized context values must not enter diagnostics.
#[allow(clippy::needless_pass_by_value)]
pub fn serialization(_error: serde_json::Error) -> RunRuntimeError {
    runtime_error("runtime context serialization failed")
}
