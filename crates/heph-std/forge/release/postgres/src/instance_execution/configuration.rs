use std::path::{Component, Path};

use release_domain::RuntimePolicy;
use release_service::InstanceExecutionError;
use serde_json::{Value, json};
use vm_trait::VmProviderOwnerScope;
use volume_domain::VolumeRootNamespaceId;

/// Checked server configuration; never a client activation or physical proof DTO.
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

    pub(super) fn json(&self) -> Value {
        json!({
            "version":1,"profile":"runtime_named_v1",
            "vm":{"namespace":self.scope.namespace(),"host_id":self.scope.host_id()},
            "volume":{"namespace":self.volume_namespace.as_uuid(),"host_id":self.scope.host_id(),"root":self.volume_root},
            "platform":{"policy":self.platform_policy,"version":self.platform_version}
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use release_domain::NetworkAccess;
    use uuid::Uuid;

    fn make(
        root: &str,
        version: &str,
    ) -> Result<InstanceExecutionConfiguration, InstanceExecutionError> {
        InstanceExecutionConfiguration::new(
            VmProviderOwnerScope::new("actual-owner".to_owned(), "host".to_owned()).unwrap(),
            root.to_owned(),
            VolumeRootNamespaceId::from_uuid(Uuid::new_v4()).unwrap(),
            RuntimePolicy {
                vcpus: 1,
                memory_mib: 512,
                network: NetworkAccess::Disabled,
            },
            version.to_owned(),
        )
    }
    #[test]
    fn configuration_bounds_and_canonical_paths_do_not_perform_io() {
        for root in [
            "/",
            "relative",
            "/tmp/../foreign",
            "/tmp//foreign",
            "/tmp/./foreign",
            "/tmp/foreign/",
            "/tmp/unsafe\n",
        ] {
            assert!(matches!(
                make(root, "platform/v1"),
                Err(InstanceExecutionError::ConfigurationConflict)
            ));
        }
        for version in ["", " padded", "unsafe\n"] {
            assert!(matches!(
                make("/prospective/nonexistent/provider", version),
                Err(InstanceExecutionError::ConfigurationConflict)
            ));
        }
        assert!(
            make("/prospective/nonexistent/provider", "platform/v1").is_ok(),
            "checked data is not physical root discovery"
        );
    }
    #[test]
    fn checked_configuration_preserves_distinct_vm_and_volume_namespaces() {
        let configured = make("/prospective/nonexistent/provider", "platform/v1").unwrap();
        let wire = configured.json();
        assert_eq!(wire["vm"]["namespace"], "actual-owner");
        assert_eq!(
            wire["volume"]["namespace"],
            configured.volume_namespace.as_uuid().to_string()
        );
        assert_eq!(wire["platform"]["version"], "platform/v1");
        assert_ne!(wire["vm"]["namespace"], wire["volume"]["namespace"]);
    }
}
