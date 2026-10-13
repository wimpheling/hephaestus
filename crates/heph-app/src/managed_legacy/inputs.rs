//! Trusted concrete inputs; no arbitrary provider or hidden-profile producer ports.

use crate::RuntimePolicy;
use control_plane_postgres::ControlPlanePool;
use heph_runtime::RootFilesystem;
use release_service::InstanceExecutionConfiguration;
use run_orchestrator::RunOperationGuards;
use run_runtime_local::LocalRunRuntimeConfig;
use secret_key_local::LocalKeyProvider;
use secret_runtime::EphemeralSecretConfig;
use std::{collections::BTreeMap, path::PathBuf, sync::Arc, time::Duration};
use vm_libkrun::{LibkrunConfig, LibkrunProvider};
use volume_local::{LocalVolumeConfig, VolumeRootOwner};
use workspace_local::LocalWorkspaceConfig;

/// The three actual distinct pools retained by the composition owner.
pub struct ManagedLegacyPools {
    /// Established owner administrator, used here only for startup preflight.
    pub admin: ControlPlanePool,
    /// Dedicated worker selecting `hephaestus_worker`.
    pub worker: ControlPlanePool,
    /// Distinct application selecting `hephaestus_app`.
    pub application: ControlPlanePool,
}

/// Host authority handoff storage and positive representable session lifetime.
pub struct ManagedLegacyAuthority {
    /// Dedicated canonical private host directory.
    pub root: PathBuf,
    /// Existing nonzero host-loaded key, never a guest credential.
    pub key: [u8; 32],
    /// Positive representable runtime session lifetime.
    pub session_ttl: Duration,
}

/// Exact supplied backend and ordinary host configuration retained privately.
///
/// The owner supplies the SAME actual provider, volume owner and guard Arcs as
/// every managed Run family. No provider is constructed, reopened or adopted.
/// Ordinary producer profile/scope are configured internally from these inputs.
pub struct ManagedLegacyInputs {
    /// SAME existing concrete managed provider.
    pub provider: Arc<LibkrunProvider>,
    /// Full trusted configuration, compared with every actual retained field.
    pub expected_provider: LibkrunConfig,
    /// SAME actual persistent-volume descriptor owner.
    pub volume_owner: Arc<VolumeRootOwner>,
    /// Checked backend/root/namespace/current-policy comparison stamp.
    pub configuration: InstanceExecutionConfiguration,
    /// Actual owner, worker and distinct application pools.
    pub pools: ManagedLegacyPools,
    /// Persistent volume, transient roots, host, formatter and lease settings.
    pub local_volume: LocalVolumeConfig,
    /// Ordinary runtime and immutable release-object roots.
    pub run_runtime: LocalRunRuntimeConfig,
    /// Actual Git workspace, result, repository, executable and quota settings.
    pub workspaces: LocalWorkspaceConfig,
    /// Host listener path that must exactly equal the provider `RuntimeGit` bridge.
    pub runtime_git_socket: PathBuf,
    /// Host broker listener path that must exactly equal the provider bridge.
    pub secret_broker_socket: PathBuf,
    /// Digest-pinned OCI references mapped to current materialized roots.
    pub root_images: BTreeMap<String, RootFilesystem>,
    /// Current ordinary runtime policy, with its exact unclamped mapping.
    pub runtime_policy: RuntimePolicy,
    /// Existing checked key provider for ordinary secret services.
    pub secret_keys: LocalKeyProvider,
    /// Actual ephemeral root, preserving the explicit memory-filesystem flag.
    pub secret_mounts: EphemeralSecretConfig,
    /// Actual host authority storage and lifetime.
    pub authority: ManagedLegacyAuthority,
    /// Supported positive scalar instance-state capacity.
    pub instance_state_capacity_bytes: u64,
    /// Positive cleanup deadline, at most the existing 30-second maximum.
    pub cleanup_timeout: Duration,
    /// SAME supplied registry installed before any orchestrator operation.
    pub operation_guards: Arc<RunOperationGuards>,
}
