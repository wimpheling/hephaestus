//! Panic-on-global-callback controls; no `PostgreSQL` or physical evidence.
use async_trait::async_trait;
use run_domain::Run;
use run_orchestrator::{
    PreparedRunAuthority, PreparedRunRuntime, PreparedRunSecrets, RunAuthorityError,
    RunAuthorityManager, RunCompletionError, RunCompletionObserver, RunRuntimeError,
    RunRuntimeManager, RunSecretError, RunSecretManager,
};
use runtime_types::RunId;
use workspace_domain::{
    PreparedRuntimeGitWorkspace, PreparedWorkspace, PublishedResult, RunWorkspaceManager,
    RuntimeGitWorkspaceManager, WorkspaceError,
};

pub struct SweepsForbidden;
#[async_trait]
impl RunRuntimeManager for SweepsForbidden {
    async fn prepare(&self, _: &Run) -> Result<PreparedRunRuntime, RunRuntimeError> {
        panic!("reconciliation must not prepare runtime");
    }
    async fn destroy(&self, _: RunId) -> Result<(), RunRuntimeError> {
        Ok(())
    }
    async fn recover(&self) -> Result<usize, RunRuntimeError> {
        panic!("global runtime recovery");
    }
}
#[async_trait]
impl RunSecretManager for SweepsForbidden {
    async fn prepare(&self, _: &Run) -> Result<PreparedRunSecrets, RunSecretError> {
        panic!("reconciliation must not prepare secrets");
    }
    async fn reauthorize(&self, _: &Run) -> Result<(), RunSecretError> {
        panic!("reconciliation must not reauthorize secrets");
    }
    async fn destroy_after_guest(&self, _: RunId) -> Result<(), RunSecretError> {
        Ok(())
    }
    async fn recover(&self) -> Result<usize, RunSecretError> {
        panic!("global secret recovery");
    }
}
#[async_trait]
impl RunAuthorityManager for SweepsForbidden {
    async fn prepare(&self, _: &Run) -> Result<PreparedRunAuthority, RunAuthorityError> {
        panic!("reconciliation must not prepare authority");
    }
    async fn reauthorize(&self, _: &Run) -> Result<(), RunAuthorityError> {
        panic!("reconciliation must not reauthorize authority");
    }
    async fn acknowledge(&self, _: &Run, _: uuid::Uuid, _: u64) -> Result<(), RunAuthorityError> {
        panic!("reconciliation must not acknowledge authority");
    }
    async fn revoke_after_guest(&self, _: RunId) -> Result<(), RunAuthorityError> {
        Ok(())
    }
    async fn recover(&self) -> Result<usize, RunAuthorityError> {
        panic!("global authority recovery");
    }
}
#[async_trait]
impl RunCompletionObserver for SweepsForbidden {
    async fn after_cleanup(&self, _: &Run) -> Result<(), RunCompletionError> {
        Ok(())
    }
    async fn recover(&self) -> Result<usize, RunCompletionError> {
        panic!("global completion recovery");
    }
}
#[async_trait]
impl RunWorkspaceManager for SweepsForbidden {
    async fn prepare(&self, _: &Run) -> Result<PreparedWorkspace, WorkspaceError> {
        panic!("reconciliation must not prepare workspace");
    }
    async fn finalize(&self, _: &Run, _: &str) -> Result<Option<PublishedResult>, WorkspaceError> {
        panic!("reconciliation must not finalize workspace");
    }
    async fn abandon(&self, _: RunId) -> Result<(), WorkspaceError> {
        Ok(())
    }
    async fn recover(&self) -> Result<usize, WorkspaceError> {
        panic!("global workspace recovery");
    }
}
#[async_trait]
impl RuntimeGitWorkspaceManager for SweepsForbidden {
    async fn prepare_runtime_git(
        &self,
        _: &Run,
    ) -> Result<Option<PreparedRuntimeGitWorkspace>, WorkspaceError> {
        panic!("reconciliation must not prepare Git workspace");
    }
    async fn abandon_runtime_git(&self, _: RunId) -> Result<(), WorkspaceError> {
        Ok(())
    }
    async fn recover_runtime_git(&self) -> Result<usize, WorkspaceError> {
        panic!("global Git recovery");
    }
}

pub fn guarded(
    orchestrator: run_orchestrator::RunOrchestrator,
) -> run_orchestrator::RunOrchestrator {
    let trap = std::sync::Arc::new(SweepsForbidden);
    orchestrator
        .with_runtime_manager(trap.clone())
        .with_secret_manager(trap.clone())
        .with_authority_manager(trap.clone())
        .with_completion_observer(trap.clone())
        .with_workspace_manager(trap.clone())
        .with_runtime_git_workspace_manager(trap)
}
