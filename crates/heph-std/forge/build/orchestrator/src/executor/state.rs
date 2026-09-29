use serde_json::Value;
use vm_trait::VmExit;

use super::{
    BTreeMap, BUILD_GUEST_ENV, BuildExecutionError, BuildExecutionResult, BuildExecutor,
    BuildRequestId, ClaimedBuild, CompleteBuild, FinalizationBuild, GuestCommand, NetworkMode,
    NetworkProfile, OUTPUT_GUEST_PATH, PathBuf, PreparedBuildWorkspace, ReleaseArtifactInput,
    ReleaseCommandKey, SOURCE_GUEST_PATH, VmId, VmMount, VmResources, VmSpec, artifact_manifest,
    cleanup_workspace, release_inputs, stored_release_inputs, validate_declared_outputs,
};

impl BuildExecutor {
    pub(super) fn active_path(&self, id: BuildRequestId) -> PathBuf {
        self.config
            .workspace_root
            .join("active")
            .join(id.to_string())
    }

    pub(super) async fn completed(
        &self,
        id: BuildRequestId,
    ) -> Result<Option<BuildExecutionResult>, BuildExecutionError> {
        self.repository
            .completed(id)
            .await?
            .map(|(release_id, release_agent_id, version, artifact_count)| {
                Ok(BuildExecutionResult {
                    build_request_id: id,
                    release_id,
                    release_agent_id,
                    release_version: version,
                    artifact_count,
                })
            })
            .transpose()
    }

    pub(super) async fn resume_finalization(
        &self,
        id: BuildRequestId,
    ) -> Result<Option<BuildExecutionResult>, BuildExecutionError> {
        let Some(finalization) = self.repository.finalization(id).await? else {
            return Ok(None);
        };
        let FinalizationBuild {
            state,
            claimed,
            artifact_manifest,
        } = finalization;
        let artifacts = if state == "sealed" {
            let output = self.active_path(id).join("output");
            let imported = self.artifacts.import_for(id.as_uuid(), &output)?;
            validate_declared_outputs(&claimed.input.build, &imported)?;
            let artifacts = release_inputs(id, &claimed.input.build, &imported)?;
            self.mark_imported(id, &artifacts).await?;
            artifacts
        } else {
            stored_release_inputs(artifact_manifest.ok_or(BuildExecutionError::StoredState)?)?
        };
        self.finish_release(claimed, artifacts).await.map(Some)
    }

    pub(super) async fn finish_release(
        &self,
        claimed: ClaimedBuild,
        artifacts: Vec<ReleaseArtifactInput>,
    ) -> Result<BuildExecutionResult, BuildExecutionError> {
        let id = claimed.input.id;
        let artifact_count = artifacts.len();
        self.releases
            .complete_build(CompleteBuild {
                command_key: ReleaseCommandKey::derive(
                    "complete-isolated-build",
                    &[id.as_uuid().as_bytes()],
                ),
                build_request_id: id,
                release_id: claimed.release_id,
                version: claimed.release_version.clone(),
                release_agent_id: claimed.release_agent_id,
                artifacts,
            })
            .await
            .map_err(|_| BuildExecutionError::Release)?;
        self.mark_drafted(id).await?;
        cleanup_workspace(&self.active_path(id))?;
        Ok(BuildExecutionResult {
            build_request_id: id,
            release_id: claimed.release_id,
            release_agent_id: claimed.release_agent_id,
            release_version: claimed.release_version,
            artifact_count,
        })
    }

    pub(super) async fn claim(
        &self,
        id: BuildRequestId,
    ) -> Result<ClaimedBuild, BuildExecutionError> {
        self.repository.claim(id).await.map_err(Into::into)
    }

    pub(super) async fn ensure_image_available(
        &self,
        id: BuildRequestId,
    ) -> Result<(), BuildExecutionError> {
        let image_reference = self.repository.image_reference(id).await?;
        let image_filesystems = self
            .config
            .image_filesystems
            .read()
            .map_err(|_| BuildExecutionError::ImageUnavailable)?;
        if image_filesystems.contains_key(&image_reference) {
            Ok(())
        } else {
            Err(BuildExecutionError::ImageUnavailable)
        }
    }

    pub(super) fn vm_spec(
        &self,
        claimed: &ClaimedBuild,
        workspace: &PreparedBuildWorkspace,
    ) -> Result<VmSpec, BuildExecutionError> {
        let build = &claimed.input.build;
        let root = {
            let image_filesystems = self
                .config
                .image_filesystems
                .read()
                // A poisoned in-process cache is never a reason to select an
                // unverified root. Treat it as unavailable until the daemon is
                // restarted from its durable manifest.
                .map_err(|_| BuildExecutionError::ImageUnavailable)?;
            image_filesystems
                .get(&claimed.input.image_reference)
                .cloned()
                .ok_or(BuildExecutionError::ImageUnavailable)?
        };
        let network = match build.network.profile {
            NetworkProfile::Disabled => NetworkMode::Disabled,
            NetworkProfile::Egress => NetworkMode::UserMode {
                ingress: Vec::new(),
            },
            NetworkProfile::BrokerOnly => return Err(BuildExecutionError::NetworkDenied),
        };
        Ok(VmSpec {
            id: VmId(format!("build-{}", claimed.input.id)),
            root,
            disks: Vec::new(),
            mounts: vec![
                VmMount {
                    tag: String::from("build-source"),
                    host_path: workspace.source.clone(),
                    guest_path: PathBuf::from(SOURCE_GUEST_PATH),
                    read_only: true,
                },
                VmMount {
                    tag: String::from("build-output"),
                    host_path: workspace.output.clone(),
                    guest_path: PathBuf::from(OUTPUT_GUEST_PATH),
                    read_only: false,
                },
            ],
            resources: VmResources {
                vcpus: build.resources.vcpus,
                memory_mib: build.resources.memory_mib,
            },
            network,
            command: GuestCommand {
                program: build.command.clone(),
                args: build.arguments.clone(),
                // The bootstrap uses this internal marker to apply the
                // descriptor limit required by compiler workloads. It is
                // removed before the user command is spawned.
                env: BTreeMap::from([(String::from(BUILD_GUEST_ENV), String::from("1"))]),
                working_dir: Some(PathBuf::from(&build.working_directory)),
            },
            runtime_authority: None,
            runtime_git_bridge: None,
            private_http_service: None,
            labels: BTreeMap::from([
                (String::from("hephaestus.kind"), String::from("build")),
                (
                    String::from("hephaestus.build-request-id"),
                    claimed.input.id.to_string(),
                ),
                (
                    String::from("hephaestus.source-ref"),
                    claimed.input.source_ref.clone(),
                ),
            ]),
        })
    }

    pub(super) async fn mark_running(&self, id: BuildRequestId) -> Result<(), BuildExecutionError> {
        self.repository.mark_running(id).await.map_err(Into::into)
    }

    pub(super) async fn mark_sealed(
        &self,
        id: BuildRequestId,
        exit: &VmExit,
        logs: &[Value],
        metrics: &[Value],
    ) -> Result<(), BuildExecutionError> {
        self.repository
            .mark_sealed(id, exit, logs, metrics)
            .await
            .map_err(Into::into)
    }

    pub(super) async fn mark_imported(
        &self,
        id: BuildRequestId,
        artifacts: &[ReleaseArtifactInput],
    ) -> Result<(), BuildExecutionError> {
        let manifest = artifact_manifest(artifacts);
        let manifest = manifest
            .as_array()
            .ok_or(BuildExecutionError::StoredState)?;
        self.repository
            .mark_imported(id, manifest)
            .await
            .map_err(Into::into)
    }

    pub(super) async fn mark_drafted(&self, id: BuildRequestId) -> Result<(), BuildExecutionError> {
        self.repository.mark_drafted(id).await.map_err(Into::into)
    }

    pub(super) async fn fail(
        &self,
        id: BuildRequestId,
        code: &str,
        logs: &[Value],
        metrics: &[Value],
    ) -> Result<(), BuildExecutionError> {
        self.repository
            .fail(id, code, None, None, logs, metrics)
            .await
            .map_err(Into::into)
    }

    pub(super) async fn fail_with_exit(
        &self,
        id: BuildRequestId,
        code: &str,
        exit: &VmExit,
        logs: &[Value],
        metrics: &[Value],
    ) -> Result<(), BuildExecutionError> {
        self.repository
            .fail(id, code, exit.code, exit.signal, logs, metrics)
            .await
            .map_err(Into::into)
    }
}
