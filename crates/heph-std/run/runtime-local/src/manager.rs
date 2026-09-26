use run_domain::{Run, RunKind};
use run_orchestrator::{PreparedRunRuntime, RunRuntimeCatalog, RunRuntimeError, RunRuntimeInput};
use runtime_types::RunId;
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::Arc,
};
use uuid::Uuid;
use vm_trait::VmMount;

use crate::{
    CONTEXT_TAG_PREFIX, MAX_RUNTIME_ARTIFACTS, MAX_RUNTIME_BYTES, PREVIOUS_RELEASE_TAG_PREFIX,
    RELEASE_TAG_PREFIX,
    artifacts::materialize_artifact,
    artifacts::write_json,
    context::{HostContext, materialize_mailbox_event},
    filesystem::{
        catalog, create_directory, filesystem, make_tree_read_only, runtime_error,
        runtime_mount_tag, validate_root,
    },
    types::{LocalGatewayReleaseRuntime, LocalRunRuntimeConfig, LocalRunRuntimeManager},
};

impl LocalRunRuntimeManager {
    /// Validates, creates, and canonicalizes the configured roots.
    ///
    /// # Errors
    ///
    /// Rejects relative, overlapping, symlink, non-directory, or
    /// group/world-writable roots.
    pub fn initialize(
        catalog: Arc<dyn RunRuntimeCatalog>,
        mut config: LocalRunRuntimeConfig,
    ) -> Result<Self, RunRuntimeError> {
        if !config.runtime_root.is_absolute() || !config.release_artifact_root.is_absolute() {
            return Err(runtime_error("runtime roots must be absolute"));
        }
        fs::create_dir_all(config.runtime_root.join("active")).map_err(filesystem)?;
        fs::create_dir_all(&config.release_artifact_root).map_err(filesystem)?;
        config.runtime_root = fs::canonicalize(config.runtime_root).map_err(filesystem)?;
        config.release_artifact_root =
            fs::canonicalize(config.release_artifact_root).map_err(filesystem)?;
        if config
            .runtime_root
            .starts_with(&config.release_artifact_root)
            || config
                .release_artifact_root
                .starts_with(&config.runtime_root)
        {
            return Err(runtime_error("runtime and release roots must not overlap"));
        }
        validate_root(&config.runtime_root)?;
        validate_root(&config.release_artifact_root)?;
        Ok(Self { catalog, config })
    }

    pub(crate) fn active_path(&self, run_id: RunId) -> PathBuf {
        self.config
            .runtime_root
            .join("active")
            .join(run_id.to_string())
    }

    /// Returns a gateway-specific release materializer over these validated
    /// runtime roots.
    #[must_use]
    pub fn gateway_release_runtime(&self) -> LocalGatewayReleaseRuntime {
        LocalGatewayReleaseRuntime {
            runtime_root: self.config.runtime_root.clone(),
            release_artifact_root: self.config.release_artifact_root.clone(),
        }
    }

    pub(crate) async fn load(&self, run: &Run) -> Result<RunRuntimeInput, RunRuntimeError> {
        let input = self.catalog.load_runtime(run).await.map_err(catalog)?;
        if input.artifacts.is_empty() || input.artifacts.len() > MAX_RUNTIME_ARTIFACTS {
            return Err(runtime_error("release artifact count is invalid"));
        }
        if run.kind == RunKind::Update && input.previous_artifacts.is_empty() {
            return Err(runtime_error("previous release artifacts are unavailable"));
        }
        if input.previous_artifacts.len() > MAX_RUNTIME_ARTIFACTS {
            return Err(runtime_error("previous release artifact count is invalid"));
        }
        Ok(input)
    }

    pub(crate) fn materialize(
        &self,
        run: &Run,
        input: &RunRuntimeInput,
    ) -> Result<PreparedRunRuntime, RunRuntimeError> {
        let staging = self
            .config
            .runtime_root
            .join(format!(".prepare-{}", Uuid::new_v4()));
        create_directory(&staging, 0o700)?;
        let result = self.materialize_staging(run, input, &staging);
        if result.is_err() {
            let _cleanup = fs::remove_dir_all(&staging);
        }
        result
    }

    // Staging the complete runtime is deliberately kept in one linear routine so
    // every validation failure shares the caller's atomic cleanup path.
    #[allow(clippy::cognitive_complexity)]
    fn materialize_staging(
        &self,
        run: &Run,
        input: &RunRuntimeInput,
        staging: &Path,
    ) -> Result<PreparedRunRuntime, RunRuntimeError> {
        let release = staging.join("release");
        let previous_release = staging.join("release-previous");
        let control = staging.join("control");
        create_directory(&release, 0o700)?;
        create_directory(&control, 0o700)?;
        let mut total = 0_u64;
        for artifact in &input.artifacts {
            total = total
                .checked_add(artifact.size_bytes)
                .ok_or_else(|| runtime_error("release artifact size is invalid"))?;
            if total > MAX_RUNTIME_BYTES {
                return Err(runtime_error("release artifact size is invalid"));
            }
            materialize_artifact(&self.config.release_artifact_root, &release, artifact)?;
        }
        tracing::debug!(run_id = %run.id, "release artifacts materialized");
        if !input.previous_artifacts.is_empty() {
            create_directory(&previous_release, 0o700)?;
            for artifact in &input.previous_artifacts {
                total = total
                    .checked_add(artifact.size_bytes)
                    .ok_or_else(|| runtime_error("release artifact size is invalid"))?;
                if total > MAX_RUNTIME_BYTES {
                    return Err(runtime_error("release artifact size is invalid"));
                }
                materialize_artifact(
                    &self.config.release_artifact_root,
                    &previous_release,
                    artifact,
                )?;
            }
            make_tree_read_only(&previous_release)?;
        }
        write_json(&control.join("parameters.json"), &input.parameters)?;
        tracing::debug!(run_id = %run.id, "runtime parameters materialized");
        if let Some(mailbox_event) = input.mailbox_event.as_ref() {
            materialize_mailbox_event(&control, mailbox_event)?;
        }
        let context = HostContext::new(run, input);
        write_json(&control.join("context.json"), &context)?;
        tracing::debug!(run_id = %run.id, "runtime context materialized");
        if let Some(parameters) = input.previous_parameters.as_ref() {
            write_json(&control.join("parameters-previous.json"), parameters)?;
        }
        make_tree_read_only(&release)?;
        tracing::debug!(run_id = %run.id, "release tree sealed");
        make_tree_read_only(&control)?;
        tracing::debug!(run_id = %run.id, "control tree sealed");
        let active = self.active_path(run.id);
        fs::rename(staging, &active).map_err(filesystem)?;
        fs::set_permissions(&active, fs::Permissions::from_mode(0o500)).map_err(filesystem)?;
        tracing::debug!(run_id = %run.id, "runtime root activated and sealed");
        let mut mounts = vec![
            VmMount {
                tag: runtime_mount_tag(RELEASE_TAG_PREFIX, run.id),
                host_path: active.join("release"),
                guest_path: PathBuf::from("/release"),
                read_only: true,
            },
            VmMount {
                tag: runtime_mount_tag(CONTEXT_TAG_PREFIX, run.id),
                host_path: active.join("control"),
                guest_path: PathBuf::from("/run/hephaestus"),
                read_only: true,
            },
        ];
        if !input.previous_artifacts.is_empty() {
            mounts.push(VmMount {
                tag: runtime_mount_tag(PREVIOUS_RELEASE_TAG_PREFIX, run.id),
                host_path: active.join("release-previous"),
                guest_path: PathBuf::from("/release-previous"),
                read_only: true,
            });
        }
        Ok(PreparedRunRuntime { mounts })
    }
}
