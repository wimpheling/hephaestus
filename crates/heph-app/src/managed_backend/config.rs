//! Trusted dedicated backend input; no discovery or environment-derived adoption.

use release_service::InstanceExecutionConfiguration;
use run_runtime_local::LocalRunRuntimeConfig;
use secret_runtime::EphemeralSecretConfig;
use std::{path::PathBuf, time::Duration};
use vm_libkrun::LibkrunConfig;
use volume_local::LocalVolumeConfig;
use workspace_local::LocalWorkspaceConfig;

/// Explicit trusted intent for one independently published physical root.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManagedRootIntent {
    /// Prepare an existing empty root, requiring observed absence of history.
    /// This prospective intent grants no adoption, database fence or Run authority.
    BootstrapEmpty,
    /// Open exact existing ownership; never initialize missing owner metadata.
    OpenExisting,
}

/// Optional dedicated managed Run backend, independent of ordinary build/OCI VMs.
///
/// The checked configuration supplies an explicit durable canonical nonnil VM
/// UUID/host and an independent explicit volume UUID/root/host. These identities
/// are comparison inputs, never actor authority or a stopped proof. Both intent
/// fields must be supplied by trusted configuration, never discovered from disk.
/// Managed startup stays pending until recovery, dispatch and shutdown are wired.
#[derive(Clone)]
pub struct ManagedBackendConfig {
    /// Full concrete native backend configuration; no arbitrary provider wrapper.
    pub provider: LibkrunConfig,
    /// Checked exact owner, volume root/namespace and current platform policy.
    pub configuration: InstanceExecutionConfiguration,
    /// Explicit intent for the dedicated VM runtime/cgroup root pair.
    pub vm_intent: ManagedRootIntent,
    /// Independent explicit intent for the persistent volume root.
    pub volume_intent: ManagedRootIntent,
    /// Dedicated local volume roots, host, formatter and positive lease lifetime.
    pub volumes: LocalVolumeConfig,
    /// Dedicated managed runtime and release-artifact roots.
    pub run_runtime: LocalRunRuntimeConfig,
    /// Dedicated workspace/results with the existing authoritative Git repository root.
    pub workspaces: LocalWorkspaceConfig,
    /// Actual listener path, exactly matching the provider `RuntimeGit` bridge.
    pub runtime_git_socket: PathBuf,
    /// Dedicated private authority handoff root.
    pub authority_root: PathBuf,
    /// Existing nonzero host-loaded authority key.
    pub authority_key: [u8; 32],
    /// Positive representable session lifetime.
    pub authority_session_ttl: Duration,
    /// Dedicated ephemeral root, preserving the required-memory-filesystem flag.
    pub secret_mounts: EphemeralSecretConfig,
    /// Positive cleanup deadline, at most the existing thirty-second bound.
    pub cleanup_timeout: Duration,
}
