use std::path::{Component, Path};

use super::InstanceExecutionError;
use release_domain::RuntimePolicy;
use serde_json::{Value, json};
use vm_trait::VmProviderOwnerScope;
use volume_domain::VolumeRootNamespaceId;

/// Checked server composition data shared by qualified execution adapters.
///
/// Construction and encoding perform no IO and confer no caller authority or
/// physical root, ownership, readiness or runtime admission proof.
#[derive(Debug, Clone)]
pub struct InstanceExecutionConfiguration {
    scope: VmProviderOwnerScope,
    volume_root: String,
    volume_namespace: VolumeRootNamespaceId,
    platform_policy: RuntimePolicy,
    platform_version: String,
}

impl InstanceExecutionConfiguration {
    /// Records exact validated provider composition and platform policy.
    ///
    /// A checked scope/namespace alone proves neither root identity nor readiness.
    /// The composition owner must validate the actual roots and providers.
    ///
    /// # Errors
    /// Rejects noncanonical/unbounded roots, invalid allocations or unsafe versions.
    pub fn new(
        scope: VmProviderOwnerScope,
        volume_root: String,
        volume_namespace: VolumeRootNamespaceId,
        platform_policy: RuntimePolicy,
        platform_version: String,
    ) -> Result<Self, InstanceExecutionError> {
        let path = Path::new(&volume_root);
        if volume_root.len() > 4096
            || !path.is_absolute()
            || volume_root == "/"
            || volume_root.chars().any(char::is_control)
            || path
                .components()
                .any(|part| matches!(part, Component::CurDir | Component::ParentDir))
            || path
                .components()
                .collect::<std::path::PathBuf>()
                .as_os_str()
                != path.as_os_str()
            || platform_version.is_empty()
            || platform_version.len() > 128
            || platform_version.trim() != platform_version
            || platform_version.chars().any(char::is_control)
            || platform_policy.vcpus == 0
            || platform_policy.memory_mib == 0
        {
            return Err(InstanceExecutionError::ConfigurationConflict);
        }
        Ok(Self {
            scope,
            volume_root,
            volume_namespace,
            platform_policy,
            platform_version,
        })
    }

    /// Returns the checked VM provider namespace and configured host.
    #[must_use]
    pub const fn provider_scope(&self) -> &VmProviderOwnerScope {
        &self.scope
    }

    /// Returns the canonical lexical volume root, without inspecting it.
    #[must_use]
    pub const fn volume_root(&self) -> &str {
        self.volume_root.as_str()
    }

    /// Returns the independent persistent volume root namespace.
    #[must_use]
    pub const fn volume_namespace(&self) -> VolumeRootNamespaceId {
        self.volume_namespace
    }

    /// Returns the exact configured platform policy, without clamping it.
    #[must_use]
    pub const fn platform_policy(&self) -> &RuntimePolicy {
        &self.platform_policy
    }

    /// Returns the exact configured platform version.
    #[must_use]
    pub const fn platform_version(&self) -> &str {
        self.platform_version.as_str()
    }

    /// Returns the exact v1 configuration stamp used by qualified admission.
    ///
    /// This pure encoder preserves the existing adapter JSON representation.
    /// Its value is metadata, never client permission or physical evidence.
    #[must_use]
    pub fn canonical_json(&self) -> Value {
        json!({
            "version":1,"profile":"runtime_named_v1",
            "vm":{"namespace":self.scope.namespace(),"host_id":self.scope.host_id()},
            "volume":{"namespace":self.volume_namespace.as_uuid(),"host_id":self.scope.host_id(),"root":self.volume_root},
            "platform":{"policy":self.platform_policy,"version":self.platform_version}
        })
    }
}

#[cfg(test)]
#[path = "configuration_tests.rs"]
mod tests;
