use std::fs;

use crate::BuildRepository;
use release_artifact_store::LocalArtifactStore;
use release_postgres::ReleaseService;
use std::sync::Arc;
use vm_trait::VmProvider;

use super::{
    BuildExecutionError, BuildExecutionResult, BuildExecutor, BuildExecutorConfig, BuildRequestId,
    filesystem, release_inputs, seal_output, validate_declared_outputs, validate_private_directory,
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
            workspace_root: super::workspace::pin::WorkspacePin::open(
                &config.workspace_root,
                None,
            )?,
            registry: Arc::new(super::owner::Registry::default()),
            config,
        })
    }
    /// Executes, seals, imports, and creates the immutable draft for one build.
    ///
    /// # Errors
    ///
    /// Returns a stable failure class after recording a durable failed build.
    /// Redelivery never launches a second VM for a nonterminal execution.
    pub async fn execute(
        &self,
        build_request_id: BuildRequestId,
    ) -> Result<BuildExecutionResult, BuildExecutionError> {
        super::owner::execute(self.clone(), build_request_id).await
    }

    pub(super) async fn execute_owned(
        &self,
        build_request_id: BuildRequestId,
        node: &Arc<super::owner::Node>,
    ) -> Result<BuildExecutionResult, BuildExecutionError> {
        node.check_open()?;
        if let Some(result) = self.completed(build_request_id).await? {
            return Ok(result);
        }
        if let Some(result) = self.resume_finalization(build_request_id, node).await? {
            return Ok(result);
        }
        self.ensure_image_available(build_request_id).await?;
        node.check_open()?;
        let claimed = self.claim(build_request_id).await?;
        let workspace = match self.materialize(&claimed, node).await {
            Ok(workspace) => workspace,
            Err(error) => {
                self.fail(build_request_id, "source_materialization", &[], &[])
                    .await?;
                return Err(error);
            }
        };
        let (exit, logs, metrics) = self.build_guest(&claimed, &workspace, node, false).await?;
        node.original_workspace()?.recheck()?;
        seal_output(&workspace.output)?;
        // Unknown sealed/imported commit must preserve exact original output.
        node.preserve_finalization();
        self.mark_sealed(build_request_id, &exit, &logs, &metrics)
            .await?;
        let imported = self
            .artifacts
            .import_for(build_request_id.as_uuid(), &workspace.output)?;
        validate_declared_outputs(&claimed.input.build, &imported)?;
        let release_inputs = release_inputs(build_request_id, &claimed.input.build, &imported)?;
        self.mark_imported(build_request_id, &release_inputs)
            .await?;
        self.finish_release(claimed, release_inputs, node).await
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
        super::owner::retry(self.clone(), build_request_id).await
    }
}
