//! Read-only checks before any initializer, including after the startup await.

use super::{AppError, ManagedLegacyInputs, component, roots};
use heph_secret::SecretMountProvider;
use release_domain::{NetworkAccess, RuntimePolicy};
use secret_runtime::FilesystemSecretMountProvider;
use std::time::Duration;
use time::OffsetDateTime;

pub(super) fn validate(inputs: &ManagedLegacyInputs) -> Result<(), AppError> {
    inputs
        .provider
        .verify_managed_configuration(
            &inputs.expected_provider,
            inputs.configuration.provider_scope(),
        )
        .map_err(component("managed Legacy managed provider binding"))?;
    inputs
        .volume_owner
        .validate()
        .map_err(component("managed Legacy physical volume owner"))?;
    let owner = &inputs.volume_owner;
    let local = &inputs.local_volume;
    let configuration = &inputs.configuration;
    if owner.root() != local.volume_root
        || owner.host() != local.host_id
        || owner.namespace_id() != configuration.volume_namespace()
        || local.volume_root.to_str() != Some(configuration.volume_root())
        || local.host_id != configuration.provider_scope().host_id()
    {
        return Err(invalid(
            "actual volume root/host/namespace differs from checked configuration",
        ));
    }
    run_domain::LegacyVmPlacementScope::new(configuration.provider_scope().clone())
        .map_err(component("managed Legacy placement scope"))?;
    validate_policy(inputs)?;
    super::workspace_validation::validate(inputs)?;
    validate_durations(inputs)?;
    roots::validate(inputs)?;
    FilesystemSecretMountProvider::new()
        .validate_config(&inputs.secret_mounts)
        .map_err(component("managed Legacy ephemeral secret root"))?;
    if inputs.authority.key == [0; 32] {
        return Err(invalid("runtime authority handoff key must be nonzero"));
    }
    if inputs.instance_state_capacity_bytes < volume_trait::MIN_LOCAL_VOLUME_CAPACITY_BYTES
        || inputs.instance_state_capacity_bytes
            > inputs.expected_provider.limits.writable_disk_max_bytes
    {
        return Err(invalid(
            "instance-state capacity is outside supported local/provider bounds",
        ));
    }
    let pools = &inputs.pools;
    if pools.admin.is_closed()
        || pools.worker.is_closed()
        || pools.application.is_closed()
        || std::ptr::eq(pools.admin.options(), pools.worker.options())
        || std::ptr::eq(pools.admin.options(), pools.application.options())
        || std::ptr::eq(pools.worker.options(), pools.application.options())
    {
        return Err(invalid("three distinct open actual pools are required"));
    }
    Ok(())
}

fn validate_policy(inputs: &ManagedLegacyInputs) -> Result<(), AppError> {
    let current = &inputs.runtime_policy;
    if current.version.is_empty()
        || current.version.trim() != current.version
        || current.version.len() > 128
        || current.version.chars().any(char::is_control)
        || current.max_vcpus == 0
        || current.max_vcpus > 8
        || current.max_memory_mib == 0
        || u64::from(current.max_memory_mib) * 1024 * 1024
            > inputs.expected_provider.limits.memory_max_bytes
        || (current.allow_broker_only && inputs.expected_provider.broker_socket_path.is_none())
    {
        return Err(invalid(
            "current runtime policy exceeds the actual provider or is invalid",
        ));
    }
    // Exact existing App mapping; no resource clamp or policy-version rewrite.
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
    let rebuilt = release_service::InstanceExecutionConfiguration::new(
        inputs.configuration.provider_scope().clone(),
        inputs
            .local_volume
            .volume_root
            .to_str()
            .ok_or_else(|| invalid("local volume root must be UTF-8"))?
            .to_owned(),
        inputs.volume_owner.namespace_id(),
        policy,
        current.version.clone(),
    )
    .map_err(component("managed Legacy checked execution configuration"))?;
    if rebuilt.canonical_json() != inputs.configuration.canonical_json() {
        return Err(invalid(
            "current exact policy/root/namespace stamp differs from checked configuration",
        ));
    }
    Ok(())
}

fn validate_durations(inputs: &ManagedLegacyInputs) -> Result<(), AppError> {
    for duration in [
        inputs.authority.session_ttl,
        inputs.local_volume.lease_duration,
    ] {
        let value = time::Duration::try_from(duration)
            .map_err(component("managed Legacy lifetime representation"))?;
        if value <= time::Duration::ZERO || OffsetDateTime::now_utc().checked_add(value).is_none() {
            return Err(invalid(
                "authority and volume lifetimes must be positive and representable",
            ));
        }
    }
    let provider = &inputs.expected_provider;
    if provider.startup_timeout.is_zero()
        || provider.readiness_timeout.is_zero()
        || provider.limits.wall_clock_timeout.is_zero()
        || inputs.cleanup_timeout.is_zero()
        || inputs.cleanup_timeout > Duration::from_secs(30)
    {
        return Err(invalid(
            "provider and cleanup timeouts must be positive and cleanup at most 30 seconds",
        ));
    }
    Ok(())
}

pub(super) fn invalid(message: &str) -> AppError {
    AppError::Configuration(format!("managed Legacy composition: {message}"))
}
