//! Ordinary Git/result/socket relationships checked without initialization.

use super::{AppError, ManagedLegacyInputs, component, paths, validation::invalid};
use std::sync::Arc;
use workspace_local::LocalWorkspaceManager;
use workspace_postgres::PgWorkspaceMetadataRepository;

pub(super) fn validate(inputs: &ManagedLegacyInputs) -> Result<(), AppError> {
    let workspace = &inputs.workspaces;
    paths::directory(&workspace.workspace_root, false, true)?;
    paths::directory(&workspace.artifact_root, false, true)?;
    paths::directory(&workspace.repository_root, false, false)?;
    paths::executable(&workspace.git_binary)?;
    paths::allowed(
        &workspace.workspace_root,
        &inputs.expected_provider.mount_roots,
    )?;
    if workspace.limits.max_file_bytes > workspace.limits.max_total_bytes {
        return Err(invalid(
            "workspace per-file quota exceeds its aggregate quota",
        ));
    }
    // Run the actual existing static validator, without its filesystem initializer.
    let metadata = Arc::new(PgWorkspaceMetadataRepository::new(
        inputs.pools.worker.clone(),
    ));
    LocalWorkspaceManager::new(metadata.clone(), metadata, workspace.clone())
        .map_err(component("managed Legacy ordinary workspace configuration"))?;
    let provider = &inputs.expected_provider;
    if provider.runtime_git_socket_path.as_ref() != Some(&inputs.runtime_git_socket)
        || provider.broker_socket_path.as_ref() != Some(&inputs.secret_broker_socket)
        || inputs.runtime_git_socket == inputs.secret_broker_socket
    {
        return Err(invalid(
            "ordinary RuntimeGit and secret broker bridges must match distinct host listener paths",
        ));
    }
    if provider.service_uid != rustix::process::geteuid().as_raw()
        || provider.service_gid != rustix::process::getegid().as_raw()
        || !provider.enforce_cgroup_v2
        || provider.limits.cpu_period_micros == 0
        || provider.limits.cpu_quota_micros == Some(0)
        || provider.limits.memory_max_bytes == 0
        || provider.limits.pids_max == 0
        || provider.limits.writable_disk_max_bytes == 0
    {
        return Err(invalid(
            "actual managed provider account/cgroup limits are incompatible",
        ));
    }
    Ok(())
}
