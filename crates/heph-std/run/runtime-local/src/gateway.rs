use run_orchestrator::{RunRuntimeArtifact, RunRuntimeError};
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf};
use uuid::Uuid;
use vm_trait::VmMount;

use crate::{
    GATEWAY_SERVICE_METADATA, GATEWAY_SERVICE_NAMESPACE, GATEWAY_SERVICE_STAGING_PREFIX,
    MAX_RUNTIME_ARTIFACTS, MAX_RUNTIME_BYTES,
    artifacts::{materialize_artifact, write_bytes, write_json},
    filesystem::{
        create_directory, ensure_existing_directory, filesystem, gateway_service_mount_tag,
        make_tree_read_only, make_tree_removable, read_service_identity, runtime_error,
        validate_sealed_service_subtree,
    },
    types::{
        GatewayServiceIdentity, GatewayServiceIdentityFile, GatewayServiceInstanceRecord,
        LocalGatewayReleaseRuntime,
    },
};

impl LocalGatewayReleaseRuntime {
    pub(crate) fn active_path(&self, invocation_id: Uuid) -> PathBuf {
        self.runtime_root
            .join("gateways")
            .join(invocation_id.to_string())
    }

    pub(crate) fn service_namespace(&self) -> PathBuf {
        self.runtime_root.join(GATEWAY_SERVICE_NAMESPACE)
    }

    pub(crate) fn service_path(&self, instance_id: Uuid) -> PathBuf {
        self.service_namespace().join(instance_id.to_string())
    }

    pub(crate) fn ensure_service_namespace(&self) -> Result<PathBuf, RunRuntimeError> {
        ensure_existing_directory(&self.runtime_root)?;
        let namespace = self.service_namespace();
        match fs::symlink_metadata(&namespace) {
            Ok(metadata)
                if metadata.file_type().is_dir()
                    && !metadata.file_type().is_symlink()
                    && metadata.permissions().mode() & 0o022 == 0 =>
            {
                Ok(namespace)
            }
            Ok(_) => Err(runtime_error("gateway service namespace is unsafe")),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                create_directory(&namespace, 0o700)?;
                Ok(namespace)
            }
            Err(error) => Err(filesystem(error)),
        }
    }

    pub(crate) fn existing_service_namespace(&self) -> Result<Option<PathBuf>, RunRuntimeError> {
        ensure_existing_directory(&self.runtime_root)?;
        let namespace = self.service_namespace();
        match fs::symlink_metadata(&namespace) {
            Ok(metadata)
                if metadata.file_type().is_dir()
                    && !metadata.file_type().is_symlink()
                    && metadata.permissions().mode() & 0o022 == 0 =>
            {
                Ok(Some(namespace))
            }
            Ok(_) => Err(runtime_error("gateway service namespace is unsafe")),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(filesystem(error)),
        }
    }

    fn validate_service_identity(identity: GatewayServiceIdentity) -> Result<(), RunRuntimeError> {
        if identity.instance_id.is_nil()
            || identity.gateway_id.is_nil()
            || identity.revision_id.is_nil()
        {
            return Err(runtime_error("gateway service identity is invalid"));
        }
        Ok(())
    }

    /// Materializes one persistent service instance in the dedicated
    /// `gateway-services` namespace. Existing instance paths fail closed;
    /// retries must use a fresh instance identity after explicit cleanup.
    ///
    /// The identity metadata is host-only and is not included in the guest
    /// mounts. Release and control trees are sealed before activation.
    ///
    /// # Errors
    ///
    /// Returns a redacted runtime error when the identity, artifact set,
    /// namespace, staging tree, or sealed materialization is invalid.
    pub fn prepare_service(
        &self,
        identity: GatewayServiceIdentity,
        artifacts: &[RunRuntimeArtifact],
        parameters: &serde_json::Value,
    ) -> Result<Vec<VmMount>, RunRuntimeError> {
        Self::validate_service_identity(identity)?;
        if artifacts.is_empty() || artifacts.len() > MAX_RUNTIME_ARTIFACTS {
            return Err(runtime_error("release artifact count is invalid"));
        }
        let parameter_bytes = serde_json::to_vec(parameters)
            .map_err(|_| runtime_error("gateway parameters are invalid"))?;
        if !parameters.is_object() || parameter_bytes.len() > 65_536 {
            return Err(runtime_error("gateway parameters exceed the object bound"));
        }
        let namespace = self.ensure_service_namespace()?;
        let active = namespace.join(identity.instance_id.to_string());
        match fs::symlink_metadata(&active) {
            Ok(_) => return Err(runtime_error("gateway service instance already exists")),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(filesystem(error)),
        }
        let staging = namespace.join(format!(
            "{GATEWAY_SERVICE_STAGING_PREFIX}{}",
            identity.instance_id
        ));
        match fs::symlink_metadata(&staging) {
            Ok(_) => return Err(runtime_error("gateway service staging already exists")),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(filesystem(error)),
        }
        create_directory(&staging, 0o700)?;
        let release = staging.join("release");
        let control = staging.join("control");
        let result = (|| {
            create_directory(&release, 0o700)?;
            let mut total = 0_u64;
            for artifact in artifacts {
                total = total
                    .checked_add(artifact.size_bytes)
                    .ok_or_else(|| runtime_error("release artifact size is invalid"))?;
                if total > MAX_RUNTIME_BYTES {
                    return Err(runtime_error("release artifact size is invalid"));
                }
                materialize_artifact(&self.release_artifact_root, &release, artifact)?;
            }
            make_tree_read_only(&release)?;
            create_directory(&control, 0o700)?;
            write_bytes(&control.join("parameters.json"), &parameter_bytes)?;
            make_tree_read_only(&control)?;
            let metadata: GatewayServiceIdentityFile = identity.into();
            write_json(&staging.join(GATEWAY_SERVICE_METADATA), &metadata)?;
            make_tree_read_only(&staging)?;
            fs::rename(&staging, &active).map_err(filesystem)?;
            fs::set_permissions(&active, fs::Permissions::from_mode(0o500)).map_err(filesystem)?;
            Ok::<_, RunRuntimeError>(vec![
                VmMount {
                    tag: gateway_service_mount_tag("gws", identity.instance_id),
                    host_path: active.join("release"),
                    guest_path: PathBuf::from("/release"),
                    read_only: true,
                },
                VmMount {
                    tag: gateway_service_mount_tag("gwt", identity.instance_id),
                    host_path: active.join("control"),
                    guest_path: PathBuf::from("/run/hephaestus"),
                    read_only: true,
                },
            ])
        })();
        if result.is_err()
            && let Ok(metadata) = fs::symlink_metadata(&staging)
            && metadata.file_type().is_dir()
            && !metadata.file_type().is_symlink()
            && make_tree_removable(&staging).is_ok()
        {
            let _cleanup = fs::remove_dir_all(&staging);
        }
        result
    }

    /// Enumerates validated active service trees in the dedicated namespace.
    /// Staging trees are handled separately by [`Self::cleanup_service_staging`].
    ///
    /// # Errors
    ///
    /// Returns a redacted runtime error when the namespace, identity metadata,
    /// or an active service tree is unsafe or malformed.
    pub fn enumerate_service_instances(
        &self,
    ) -> Result<Vec<GatewayServiceInstanceRecord>, RunRuntimeError> {
        let Some(namespace) = self.existing_service_namespace()? else {
            return Ok(Vec::new());
        };
        let entries = fs::read_dir(&namespace).map_err(filesystem)?;
        let mut records = Vec::new();
        for entry in entries {
            let entry = entry.map_err(filesystem)?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| runtime_error("gateway service entry is invalid"))?;
            if name.starts_with(GATEWAY_SERVICE_STAGING_PREFIX) {
                continue;
            }
            let instance_id = Uuid::parse_str(&name)
                .map_err(|_| runtime_error("gateway service entry is invalid"))?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path).map_err(filesystem)?;
            if !metadata.file_type().is_dir()
                || metadata.file_type().is_symlink()
                || metadata.permissions().mode() & 0o222 != 0
            {
                return Err(runtime_error("gateway service tree is unsafe"));
            }
            let identity = read_service_identity(&path.join(GATEWAY_SERVICE_METADATA))?;
            if identity.instance_id != instance_id {
                return Err(runtime_error(
                    "gateway service identity does not match path",
                ));
            }
            validate_sealed_service_subtree(&path.join("release"))?;
            validate_sealed_service_subtree(&path.join("control"))?;
            records.push(GatewayServiceInstanceRecord { identity, path });
        }
        records.sort_by_key(|record| record.identity.instance_id);
        Ok(records)
    }

    /// Destroys one service tree only after its exact sealed identity matches.
    /// Callers must confirm provider ownership is quiescent before invoking it.
    ///
    /// # Errors
    ///
    /// Returns a redacted runtime error when the identity does not match or
    /// the selected tree is unsafe to remove.
    pub fn destroy_service(&self, identity: GatewayServiceIdentity) -> Result<(), RunRuntimeError> {
        Self::validate_service_identity(identity)?;
        let _namespace = self.ensure_service_namespace()?;
        let active = self.service_path(identity.instance_id);
        match fs::symlink_metadata(&active) {
            Ok(metadata) => {
                if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
                    return Err(runtime_error("gateway service tree is unsafe"));
                }
                if read_service_identity(&active.join(GATEWAY_SERVICE_METADATA))? != identity {
                    return Err(runtime_error("gateway service identity mismatch"));
                }
                make_tree_removable(&active)?;
                fs::remove_dir_all(active).map_err(filesystem)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(filesystem(error)),
        }
    }

    /// Removes only stale staging trees below `gateway-services`.
    ///
    /// The service supervisor must call this only after establishing that no
    /// materialization operation is still running.
    ///
    /// # Errors
    ///
    /// Returns a redacted runtime error when the namespace or a staging tree
    /// is unsafe or malformed.
    pub fn cleanup_service_staging(&self) -> Result<usize, RunRuntimeError> {
        let Some(namespace) = self.existing_service_namespace()? else {
            return Ok(0);
        };
        let entries = fs::read_dir(&namespace).map_err(filesystem)?;
        let mut cleaned = 0;
        for entry in entries {
            let entry = entry.map_err(filesystem)?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| runtime_error("gateway service staging entry is invalid"))?;
            let Some(instance) = name.strip_prefix(GATEWAY_SERVICE_STAGING_PREFIX) else {
                continue;
            };
            Uuid::parse_str(instance)
                .map_err(|_| runtime_error("gateway service staging entry is invalid"))?;
            let staging = entry.path();
            make_tree_removable(&staging)?;
            fs::remove_dir_all(staging).map_err(filesystem)?;
            cleaned += 1;
        }
        Ok(cleaned)
    }
}
