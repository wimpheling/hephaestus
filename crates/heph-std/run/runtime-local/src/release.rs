use run_orchestrator::{RunRuntimeArtifact, RunRuntimeError};
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf};
use uuid::Uuid;
use vm_trait::VmMount;

use crate::{
    MAX_RUNTIME_ARTIFACTS, MAX_RUNTIME_BYTES,
    artifacts::{materialize_artifact, write_bytes},
    filesystem::{
        create_directory, filesystem, gateway_runtime_mount_tag, make_tree_read_only,
        make_tree_removable, runtime_error,
    },
    types::LocalGatewayReleaseRuntime,
};

impl LocalGatewayReleaseRuntime {
    /// Materializes verified artifacts and exact revision parameters into
    /// read-only `/release` and `/run/hephaestus` trees.
    ///
    /// # Errors
    ///
    /// Returns a redacted error when artifact metadata, the canonical store,
    /// or the local runtime tree is invalid.
    pub fn prepare(
        &self,
        invocation_id: Uuid,
        artifacts: &[RunRuntimeArtifact],
        parameters: &serde_json::Value,
    ) -> Result<Vec<VmMount>, RunRuntimeError> {
        if artifacts.is_empty() || artifacts.len() > MAX_RUNTIME_ARTIFACTS {
            return Err(runtime_error("release artifact count is invalid"));
        }
        let parameter_bytes = serde_json::to_vec(parameters)
            .map_err(|_| runtime_error("gateway parameters are invalid"))?;
        if !parameters.is_object() || parameter_bytes.len() > 65_536 {
            return Err(runtime_error("gateway parameters exceed the object bound"));
        }
        let gateways = self.runtime_root.join("gateways");
        fs::create_dir_all(&gateways).map_err(filesystem)?;
        fs::set_permissions(&gateways, fs::Permissions::from_mode(0o700)).map_err(filesystem)?;
        let active = self.active_path(invocation_id);
        if active.exists() {
            return Err(runtime_error("gateway runtime already exists"));
        }
        let staging = self
            .runtime_root
            .join(format!(".gateway-prepare-{invocation_id}"));
        create_directory(&staging, 0o700)?;
        let release = staging.join("release");
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
            let control = staging.join("control");
            create_directory(&control, 0o700)?;
            write_bytes(&control.join("parameters.json"), &parameter_bytes)?;
            make_tree_read_only(&control)?;
            fs::rename(&staging, &active).map_err(filesystem)?;
            fs::set_permissions(&active, fs::Permissions::from_mode(0o500)).map_err(filesystem)?;
            Ok::<_, RunRuntimeError>(vec![
                VmMount {
                    tag: gateway_runtime_mount_tag(invocation_id),
                    host_path: active.join("release"),
                    guest_path: PathBuf::from("/release"),
                    read_only: true,
                },
                VmMount {
                    tag: format!("gwp-{}", invocation_id.simple()),
                    host_path: active.join("control"),
                    guest_path: PathBuf::from("/run/hephaestus"),
                    read_only: true,
                },
            ])
        })();
        if result.is_err() {
            let _cleanup = fs::remove_dir_all(&staging);
        }
        result
    }

    /// Removes one gateway release tree after its VM has been destroyed.
    ///
    /// # Errors
    ///
    /// Returns a redacted error when the path is not a safe local runtime tree.
    pub fn destroy(&self, invocation_id: Uuid) -> Result<(), RunRuntimeError> {
        let active = self.active_path(invocation_id);
        match fs::symlink_metadata(&active) {
            Ok(_) => {
                make_tree_removable(&active)?;
                fs::remove_dir_all(active).map_err(filesystem)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(filesystem(error)),
        }
    }
}
