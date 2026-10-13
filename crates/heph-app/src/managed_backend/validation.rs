//! Native and exact controller-input checks before root publication or initializers.

use super::{AppConfig, AppError, ManagedBackendConfig, custody, invalid, roots};
use crate::VmBackendConfig;
use heph_secret::SecretMountProvider;
use release_domain::{NetworkAccess, RuntimePolicy};
use release_service::InstanceExecutionConfiguration;
use secret_runtime::FilesystemSecretMountProvider;
use std::{sync::Arc, time::Duration};
use time::OffsetDateTime;
use vm_libkrun::LibkrunProvider;
use volume_local::LocalVolumeStore;
use volume_postgres::PostgresVolumeMetadataRepository;
use workspace_local::LocalWorkspaceManager;
use workspace_postgres::PgWorkspaceMetadataRepository;

pub(super) fn validate(app: &AppConfig, config: &ManagedBackendConfig) -> Result<(), AppError> {
    inspectable(&app.vm_backend)?;
    namespace(config.configuration.provider_scope().namespace())?;
    if let VmBackendConfig::Libkrun(ordinary) = &app.vm_backend {
        LibkrunProvider::validate_configuration(ordinary).map_err(custody::failure)?;
    }
    if config.volumes.host_id != config.configuration.provider_scope().host_id()
        || config.volumes.volume_root.to_str() != Some(config.configuration.volume_root())
        || config.workspaces.repository_root != app.repository_root
        || config.provider.runtime_git_socket_path.as_ref() != Some(&config.runtime_git_socket)
        || config.provider.broker_socket_path.as_ref() != Some(&app.secret_broker_socket)
        || config.runtime_git_socket == app.secret_broker_socket
    {
        return Err(invalid(
            "managed host/root/repository/listeners differ from trusted inputs",
        ));
    }
    if let VmBackendConfig::Libkrun(ordinary) = &app.vm_backend {
        if ordinary.runtime_git_socket_path.as_ref() != Some(&config.runtime_git_socket) {
            return Err(invalid(
                "ordinary and managed RuntimeGit listener paths differ",
            ));
        }
    }
    policy(app, config)?;
    durations(config)?;
    roots::validate(app, config)?;
    FilesystemSecretMountProvider::new()
        .validate_config(&app.secret_mounts)
        .map_err(custody::failure)?;
    FilesystemSecretMountProvider::new()
        .validate_config(&config.secret_mounts)
        .map_err(custody::failure)?;
    if config.authority_key == [0; 32]
        || app.agent_state_capacity_bytes < volume_trait::MIN_LOCAL_VOLUME_CAPACITY_BYTES
        || app.agent_state_capacity_bytes > config.provider.limits.writable_disk_max_bytes
        || !config.provider.enforce_cgroup_v2
        || config.provider.limits.cpu_quota_micros == Some(0)
        || config.provider.limits.writable_disk_max_bytes == 0
    {
        return Err(invalid("invalid managed authority/capacity/cgroup limits"));
    }
    // Public complete native validator; no backend load, owner publication or VM.
    LibkrunProvider::validate_configuration(&config.provider).map_err(custody::failure)
}

pub(super) fn namespace(value: &str) -> Result<(), AppError> {
    let parsed = uuid::Uuid::parse_str(value).map_err(custody::failure)?;
    if parsed.is_nil() || parsed.to_string() != value {
        return Err(invalid(
            "VM namespace must be an explicit canonical nonnil UUID",
        ));
    }
    Ok(())
}

fn policy(app: &AppConfig, config: &ManagedBackendConfig) -> Result<(), AppError> {
    let current = &app.runtime_policy;
    if current.max_vcpus == 0
        || current.max_vcpus > 8
        || current.max_memory_mib == 0
        || u64::from(current.max_memory_mib) * 1024 * 1024 > config.provider.limits.memory_max_bytes
        || (current.allow_broker_only && config.provider.broker_socket_path.is_none())
    {
        return Err(invalid(
            "current runtime policy exceeds the dedicated provider",
        ));
    }
    let policy = RuntimePolicy {
        vcpus: current.max_vcpus,
        memory_mib: current.max_memory_mib,
        network: if current.allow_egress {
            NetworkAccess::Egress
        } else if current.allow_broker_only {
            NetworkAccess::BrokerOnly
        } else {
            NetworkAccess::Disabled
        },
    };
    let rebuilt = InstanceExecutionConfiguration::new(
        config.configuration.provider_scope().clone(),
        config.configuration.volume_root().to_owned(),
        config.configuration.volume_namespace(),
        policy,
        current.version.clone(),
    )
    .map_err(custody::failure)?;
    if rebuilt.canonical_json() != config.configuration.canonical_json() {
        return Err(invalid(
            "managed checked stamp differs from current exact runtime policy",
        ));
    }
    Ok(())
}

fn durations(config: &ManagedBackendConfig) -> Result<(), AppError> {
    for duration in [config.authority_session_ttl, config.volumes.lease_duration] {
        let value = time::Duration::try_from(duration).map_err(custody::failure)?;
        if value <= time::Duration::ZERO || OffsetDateTime::now_utc().checked_add(value).is_none() {
            return Err(invalid(
                "managed session/lease lifetimes must be positive and representable",
            ));
        }
    }
    if config.provider.startup_timeout.is_zero()
        || config.provider.readiness_timeout.is_zero()
        || config.provider.limits.wall_clock_timeout.is_zero()
        || config.cleanup_timeout.is_zero()
        || config.cleanup_timeout > Duration::from_secs(30)
    {
        return Err(invalid(
            "managed timeouts must be positive and cleanup at most thirty seconds",
        ));
    }
    Ok(())
}

pub(super) fn repositories(
    app: &AppConfig,
    config: &ManagedBackendConfig,
    worker: &control_plane_postgres::ControlPlanePool,
) -> Result<(), AppError> {
    // Actual existing constructor validators only: no initialization and no SQL.
    for volume in [&app.volumes, &config.volumes] {
        LocalVolumeStore::new(
            Arc::new(PostgresVolumeMetadataRepository::new(worker.clone())),
            volume.clone(),
        )
        .map_err(custody::failure)?;
    }
    for workspace in [&app.workspaces, &config.workspaces] {
        let metadata = Arc::new(PgWorkspaceMetadataRepository::new(worker.clone()));
        LocalWorkspaceManager::new(metadata.clone(), metadata, workspace.clone())
            .map_err(custody::failure)?;
    }
    Ok(())
}

pub(super) fn inspectable(backend: &VmBackendConfig) -> Result<(), AppError> {
    match backend {
        VmBackendConfig::Custom(_) | VmBackendConfig::CustomWithRuntimeGitSocket { .. } => Err(
            invalid("custom ordinary providers have uninspectable mutable roots"),
        ),
        VmBackendConfig::Fake | VmBackendConfig::FixtureResult | VmBackendConfig::Libkrun(_) => {
            Ok(())
        }
    }
}
