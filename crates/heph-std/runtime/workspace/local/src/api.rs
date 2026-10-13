use crate::LocalWorkspaceManager;
use crate::common::{
    PreparedRuntimeGitWorkspace, PreparedWorkspace, PublishedResult, Run, RunId,
    RunWorkspaceManager, RuntimeGitWorkspaceManager, WorkspaceError,
};
use async_trait::async_trait;

#[async_trait]
impl RunWorkspaceManager for LocalWorkspaceManager {
    async fn prepare(&self, run: &Run) -> Result<PreparedWorkspace, WorkspaceError> {
        reject_invocation(run)?;
        self.prepare_run(run)
            .await
            .map_err(WorkspaceError::operation)
    }

    async fn finalize(
        &self,
        run: &Run,
        message: &str,
    ) -> Result<Option<PublishedResult>, WorkspaceError> {
        reject_invocation(run)?;
        self.finalize_run(run, message)
            .await
            .map_err(WorkspaceError::operation)
    }

    async fn abandon(&self, run_id: RunId) -> Result<(), WorkspaceError> {
        self.abandon_run(run_id)
            .await
            .map_err(WorkspaceError::operation)
    }

    async fn recover(&self) -> Result<usize, WorkspaceError> {
        self.recover_incomplete()
            .await
            .map_err(WorkspaceError::operation)
    }
}

#[async_trait]
impl RuntimeGitWorkspaceManager for LocalWorkspaceManager {
    async fn prepare_runtime_git(
        &self,
        run: &Run,
    ) -> Result<Option<PreparedRuntimeGitWorkspace>, WorkspaceError> {
        reject_invocation(run)?;
        self.prepare_runtime_git_run(run)
            .await
            .map_err(WorkspaceError::operation)
    }

    async fn abandon_runtime_git(&self, run_id: RunId) -> Result<(), WorkspaceError> {
        self.abandon_runtime_git_run(run_id)
            .await
            .map_err(WorkspaceError::operation)
    }

    async fn recover_runtime_git(&self) -> Result<usize, WorkspaceError> {
        self.recover_runtime_git_runs()
            .await
            .map_err(WorkspaceError::operation)
    }
}

fn reject_invocation(run: &Run) -> Result<(), WorkspaceError> {
    if run.kind == run_domain::RunKind::Invocation {
        return Err(WorkspaceError::operation(
            crate::LocalWorkspaceError::State(String::from(
                "Invocation repository workspaces are unsupported",
            )),
        ));
    }
    Ok(())
}
