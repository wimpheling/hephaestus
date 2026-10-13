//! One concrete shared tuple plus the actual ordinary-only host configuration.

use crate::{
    ManagedInvocationInputs, ManagedLegacyAuthority, ManagedLegacyInputs, ManagedLegacyPools,
};
use std::path::PathBuf;
use workspace_local::LocalWorkspaceConfig;

/// Trusted prospective policy applied only to a positively observed unborn request.
///
/// Persisted original profile always wins. This policy grants no IO or actor
/// authority; the actual protected Legacy birth independently checks the request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManagedOrdinaryAdmissionPolicy {
    /// Preserve ordinary compatibility through strict protected Legacy birth.
    /// Canonical admission is unsupported in this bounded executor.
    LegacyCompatibility,
}

/// Concrete inputs supplied once for both actual managed controller families.
///
/// `shared` contains the SAME typed provider, full expected provider configuration,
/// volume owner, checked execution configuration, three actual pools and operation
/// guards, plus their common host runtime/policy/root/secret/authority settings.
/// Ordinary inputs are derived internally; no second backend tuple can be supplied.
/// The explicit ephemeral memory-filesystem flag and all trusted values are copied
/// exactly. Historical actor/request identifiers are never constructor identities.
pub struct ManagedRunCommandInputs {
    /// Existing concrete input tuple, retained by the Invocation owner and cloned
    /// field-for-field into the actual ordinary owner and immutable reader.
    pub shared: ManagedInvocationInputs,
    /// Actual ordinary Git workspace, result, repository, executable and quotas.
    pub workspaces: LocalWorkspaceConfig,
    /// Actual host listener path checked against the shared provider Git bridge.
    pub runtime_git_socket: PathBuf,
    /// Actual host listener path checked against the shared provider broker bridge.
    pub secret_broker_socket: PathBuf,
    /// Explicit future-birth choice; never overrides any persisted original.
    pub ordinary_policy: ManagedOrdinaryAdmissionPolicy,
}

impl ManagedRunCommandInputs {
    pub(super) fn legacy_inputs(&self) -> ManagedLegacyInputs {
        let shared = &self.shared;
        ManagedLegacyInputs {
            provider: shared.provider.clone(),
            expected_provider: shared.expected_provider.clone(),
            volume_owner: shared.volume_owner.clone(),
            configuration: shared.configuration.clone(),
            pools: ManagedLegacyPools {
                admin: shared.pools.admin.clone(),
                worker: shared.pools.worker.clone(),
                application: shared.pools.application.clone(),
            },
            local_volume: shared.local_volume.clone(),
            run_runtime: shared.run_runtime.clone(),
            workspaces: self.workspaces.clone(),
            runtime_git_socket: self.runtime_git_socket.clone(),
            secret_broker_socket: self.secret_broker_socket.clone(),
            root_images: shared.root_images.clone(),
            runtime_policy: shared.runtime_policy.clone(),
            secret_keys: shared.secret_keys.clone(),
            secret_mounts: shared.secret_mounts.clone(),
            authority: ManagedLegacyAuthority {
                root: shared.authority.root.clone(),
                key: shared.authority.key,
                session_ttl: shared.authority.session_ttl,
            },
            instance_state_capacity_bytes: shared.instance_state_capacity_bytes,
            cleanup_timeout: shared.cleanup_timeout,
            operation_guards: shared.operation_guards.clone(),
        }
    }
}
