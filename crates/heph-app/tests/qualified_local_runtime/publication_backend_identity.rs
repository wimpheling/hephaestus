//! Identity checks extracted from the retained publication owner.

use std::sync::Arc;

use hephaestus_app::{AppConfig, VmBackendConfig};
use release_service::InstanceExecutionConfiguration;
use run_runtime_local::LocalRunRuntimeConfig;
use vm_trait::VmProvider;
use volume_local::LocalVolumeConfig;

use super::{
    BuildError, PublicationBackend, PublicationProvider, PublishedVariant, publication, unavailable,
};

impl PublicationBackend {
    pub(super) fn validate(&self) -> Result<(), BuildError> {
        self.allocation.validate_root()?;
        assert!(!self.pool()?.is_closed());
        let provider = {
            let actual: Arc<dyn VmProvider> = self
                .actual_provider
                .as_ref()
                .ok_or_else(unavailable)?
                .clone();
            let provider = self.provider()?;
            assert!(provider.retains_provider(&actual));
            provider
        };
        assert_eq!(provider.retained_scope(), &self.inputs.provider_scope);
        let owner = self.owner.as_ref().ok_or_else(unavailable)?;
        let local = self.local.as_ref().ok_or_else(unavailable)?;
        let runtime = self.runtime.as_ref().ok_or_else(unavailable)?;
        let configuration = self.configuration.as_ref().ok_or_else(unavailable)?;
        publication::validate_identity(local, owner, provider.as_ref(), configuration);
        let app = self.app_config.as_ref().ok_or_else(unavailable)?;
        self.validate_injection(app, provider)?;
        Self::validate_runtime(app, local, runtime);
        self.validate_policy_and_roots(app, local, configuration)
    }
    fn validate_injection(
        &self,
        app: &AppConfig,
        provider: &Arc<PublicationProvider>,
    ) -> Result<(), BuildError> {
        let VmBackendConfig::CustomWithRuntimeGitSocket {
            provider: injected,
            runtime_git_socket_path,
        } = &app.vm_backend
        else {
            return Err(unavailable());
        };
        let retained: Arc<dyn VmProvider> = provider.clone();
        assert!(Arc::ptr_eq(injected, &retained));
        assert_eq!(
            runtime_git_socket_path,
            &self.allocation.root.join("runtime-git.sock")
        );
        assert_eq!(
            app.secret_broker_socket,
            self.allocation.root.join("secret-broker.sock")
        );
        assert_eq!(app.database_url, self.allocation.database_url);
        Ok(())
    }
    fn validate_runtime(
        app: &AppConfig,
        local: &LocalVolumeConfig,
        runtime: &LocalRunRuntimeConfig,
    ) {
        assert_eq!(app.volumes.volume_root, local.volume_root);
        assert_eq!(app.volumes.host_id, local.host_id);
        assert_eq!(
            app.volumes.transient_runtime_roots,
            local.transient_runtime_roots
        );
        assert_eq!(app.volumes.lease_duration, local.lease_duration);
        assert_eq!(app.volumes.mkfs_ext4, local.mkfs_ext4);
        assert_eq!(app.run_runtime.runtime_root, runtime.runtime_root);
        assert_eq!(
            app.run_runtime.release_artifact_root,
            runtime.release_artifact_root
        );
    }
    fn validate_policy_and_roots(
        &self,
        app: &AppConfig,
        local: &LocalVolumeConfig,
        configuration: &InstanceExecutionConfiguration,
    ) -> Result<(), BuildError> {
        assert_eq!(
            configuration.canonical_json(),
            self.execution_configuration(local)?.canonical_json()
        );
        assert_eq!(app.runtime_policy.version, self.inputs.policy.version);
        assert_eq!(app.runtime_policy.max_vcpus, self.inputs.policy.max_vcpus);
        assert_eq!(
            app.runtime_policy.max_memory_mib,
            self.inputs.policy.max_memory_mib
        );
        assert_eq!(
            app.runtime_policy.allow_egress,
            self.inputs.policy.allow_egress
        );
        assert_eq!(
            app.runtime_policy.allow_broker_only,
            self.inputs.policy.allow_broker_only
        );
        assert_eq!(app.root_images.len(), 1);
        let Some(vm_trait::RootFilesystem::Directory { host_path }) =
            app.root_images.get(self.allocation.image_reference())
        else {
            return Err(unavailable());
        };
        assert_eq!(host_path, &self.inputs.rootfs);
        assert_eq!(
            app.build_workspace_root,
            self.allocation.root.join("isolated-builds")
        );
        assert_eq!(
            app.workspaces.workspace_root,
            self.allocation.root.join("workspaces")
        );
        assert_eq!(
            app.workspaces.repository_root,
            self.allocation.root.join("repositories")
        );
        assert_eq!(
            app.workspaces.artifact_root,
            self.allocation.root.join("artifacts")
        );
        assert!(app.gateway_edge.is_none() && app.oci_builder.is_none());
        Ok(())
    }
}

impl PublicationBackend {
    pub(super) fn validate_pair(
        &self,
        pair: &[Arc<PublishedVariant>; 2],
    ) -> Result<(), BuildError> {
        self.validate()?;
        let [zero, two] = pair;
        if zero.variant != super::VolumeVariant::Zero
            || two.variant != super::VolumeVariant::Two
            || zero.project_id() != two.project_id()
            || zero.actor.user_id != two.actor.user_id
            || zero.published.repository_id == two.published.repository_id
            || zero.published.build_request_id == two.published.build_request_id
            || zero.published.release_id == two.published.release_id
            || zero.published.release_agent_id == two.published.release_agent_id
        {
            return Err(unavailable());
        }
        let injected: Arc<dyn VmProvider> = self.provider()?.clone();
        let owner = self.owner.as_ref().ok_or_else(unavailable)?;
        let seed = self.seed.as_ref().ok_or_else(unavailable)?;
        let configuration = self.configuration.as_ref().ok_or_else(unavailable)?;
        for p in pair {
            p.validate_identity();
            Self::validate_runtime(
                self.app_config.as_ref().ok_or_else(unavailable)?,
                &p.local,
                &p.runtime,
            );
            if !Arc::ptr_eq(&p.provider, &injected)
                || !Arc::ptr_eq(&p.owner, owner)
                || p.project_id() != seed.project
                || p.actor.user_id != seed.actor.user_id
                || p.configuration.canonical_json() != configuration.canonical_json()
                || !std::ptr::eq(p.pool.options(), self.pool()?.options())
                || !Arc::ptr_eq(&p.pool.connect_options(), &self.pool()?.connect_options())
            {
                return Err(unavailable());
            }
        }
        Ok(())
    }
}
