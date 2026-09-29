use std::fs;

use crate::BuildRepository;
use release_artifact_store::LocalArtifactStore;
use release_postgres::ReleaseService;
use std::sync::Arc;
use vm_trait::VmProvider;

use super::{
    BuildExecutionError, BuildExecutionResult, BuildExecutor, BuildExecutorConfig, BuildRequestId,
    StopMode, cleanup_workspace, collect_execution, filesystem, prepare_workspace, release_inputs,
    seal_output, validate_declared_outputs, validate_private_directory,
};

impl BuildExecutor {
    /// Validates and creates private transient roots.
    ///
    /// # Errors
    ///
    /// Rejects relative, overlapping, unsafe, or missing trusted paths.
    pub fn initialize(
        repository: Arc<dyn BuildRepository>,
        provider: Arc<dyn VmProvider>,
        artifacts: LocalArtifactStore,
        releases: Arc<ReleaseService>,
        mut config: BuildExecutorConfig,
    ) -> Result<Self, BuildExecutionError> {
        if !config.workspace_root.is_absolute()
            || !config.repository_root.is_absolute()
            || !config.git_binary.is_absolute()
            || config.timeout.is_zero()
        {
            return Err(BuildExecutionError::InvalidConfiguration);
        }
        fs::create_dir_all(config.workspace_root.join("active")).map_err(filesystem)?;
        config.workspace_root = fs::canonicalize(config.workspace_root).map_err(filesystem)?;
        config.repository_root = fs::canonicalize(config.repository_root).map_err(filesystem)?;
        config.git_binary = fs::canonicalize(config.git_binary).map_err(filesystem)?;
        if config.workspace_root.starts_with(&config.repository_root)
            || config.repository_root.starts_with(&config.workspace_root)
            || !config.git_binary.is_file()
        {
            return Err(BuildExecutionError::InvalidConfiguration);
        }
        validate_private_directory(&config.workspace_root)?;
        Ok(Self {
            repository,
            provider,
            artifacts,
            releases,
            config,
        })
    }
    /// Executes, seals, imports, and creates the immutable draft for one build.
    ///
    /// # Errors
    ///
    /// Returns a stable failure class after recording a durable failed build.
    /// Redelivery never launches a second VM for a nonterminal execution.
    #[allow(clippy::too_many_lines)]
    pub async fn execute(
        &self,
        build_request_id: BuildRequestId,
    ) -> Result<BuildExecutionResult, BuildExecutionError> {
        if let Some(result) = self.completed(build_request_id).await? {
            return Ok(result);
        }
        if let Some(result) = self.resume_finalization(build_request_id).await? {
            return Ok(result);
        }
        self.ensure_image_available(build_request_id).await?;
        let claimed = self.claim(build_request_id).await?;
        let workspace = self.active_path(build_request_id);
        let input = claimed.input.clone();
        let materializer = self.config.clone();
        let prepared = tokio::task::spawn_blocking(move || {
            prepare_workspace(&materializer, &input, &workspace)
        })
        .await
        .map_err(|_| BuildExecutionError::WorkerJoin)?
        .inspect_err(|_| {
            tracing::warn!(%build_request_id, "exact build source materialization failed");
        });
        let workspace = match prepared {
            Ok(workspace) => workspace,
            Err(error) => {
                self.fail(build_request_id, "source_materialization", &[], &[])
                    .await?;
                return Err(error);
            }
        };
        let spec = match self.vm_spec(&claimed, &workspace) {
            Ok(spec) => spec,
            Err(error) => {
                self.fail(build_request_id, "invalid_build_contract", &[], &[])
                    .await?;
                cleanup_workspace(&workspace.root)?;
                return Err(error);
            }
        };
        let Ok(instance) = self.provider.provision(spec).await else {
            self.fail(build_request_id, "vm_provision", &[], &[])
                .await?;
            cleanup_workspace(&workspace.root)?;
            return Err(BuildExecutionError::Vm);
        };
        let mut events = instance.subscribe_events();
        if instance.start().await.is_err() {
            drop(instance.destroy().await);
            self.fail(build_request_id, "vm_start", &[], &[]).await?;
            cleanup_workspace(&workspace.root)?;
            return Err(BuildExecutionError::Vm);
        }
        self.mark_running(build_request_id).await?;
        let (exit, logs, metrics, timed_out) =
            collect_execution(&instance, &mut events, self.config.timeout).await;
        if timed_out {
            drop(instance.stop(StopMode::Force).await);
        }
        if instance.destroy().await.is_err() {
            self.fail(build_request_id, "vm_destroy", &logs, &metrics)
                .await?;
            return Err(BuildExecutionError::VmCleanup);
        }
        let exit = exit.ok_or(BuildExecutionError::Vm)?;
        if timed_out || exit.code != Some(0) || exit.signal.is_some() {
            self.fail_with_exit(build_request_id, "guest_failed", &exit, &logs, &metrics)
                .await?;
            cleanup_workspace(&workspace.root)?;
            return Err(BuildExecutionError::GuestFailed);
        }
        seal_output(&workspace.output)?;
        self.mark_sealed(build_request_id, &exit, &logs, &metrics)
            .await?;
        let imported = self
            .artifacts
            .import_for(build_request_id.as_uuid(), &workspace.output)?;
        validate_declared_outputs(&claimed.input.build, &imported)?;
        let release_inputs = release_inputs(build_request_id, &claimed.input.build, &imported)?;
        self.mark_imported(build_request_id, &release_inputs)
            .await?;
        self.finish_release(claimed, release_inputs).await
    }

    /// Resets one failed execution through the trusted worker boundary and
    /// runs the next durable attempt with the same immutable inputs.
    ///
    /// # Errors
    ///
    /// Returns a durable repository, execution, or release-finalization
    /// failure.
    pub async fn retry(
        &self,
        build_request_id: BuildRequestId,
    ) -> Result<BuildExecutionResult, BuildExecutionError> {
        self.ensure_image_available(build_request_id).await?;
        self.repository.reset_for_retry(build_request_id).await?;
        self.execute(build_request_id).await
    }
}
