use async_trait::async_trait;
use run_domain::Run;
use run_domain::{RunKind, RunOutcome, RunState};
use run_orchestrator::{
    PreparedRunRuntime, RunCompletionError, RunCompletionObserver, RunRuntimeError,
    RunRuntimeManager,
};
use runtime_types::{
    AgentInstanceId, AgentInstanceRevisionId, CommandId, ReleaseAgentId, ReleaseId, RunId,
};
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex as StdMutex,
        atomic::{AtomicUsize, Ordering},
    },
};
use time::OffsetDateTime;
use uuid::Uuid;
use vm_trait::VmMount;
use workspace_domain::{
    PreparedRuntimeGitWorkspace, PreparedWorkspace, PublishedResult, RunWorkspaceManager,
    RuntimeGitWorkspaceManager, WorkspaceError, WorkspaceId,
};

use super::{
    fakes::{
        RecordingRuntimeGitWorkspaceManager, RecordingRuntimeManager, RecordingWorkspaceManager,
    },
    helpers::lock,
};

pub struct RecordingCompletion {
    pub log: Arc<StdMutex<Vec<&'static str>>>,
}

pub struct CountingCompletion {
    pub completions: AtomicUsize,
    pub recovered: usize,
}

impl CountingCompletion {
    pub const fn new(recovered: usize) -> Self {
        Self {
            completions: AtomicUsize::new(0),
            recovered,
        }
    }
}

#[async_trait]
impl RunCompletionObserver for CountingCompletion {
    async fn after_cleanup(&self, _run: &Run) -> Result<(), RunCompletionError> {
        self.completions.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    async fn recover(&self) -> Result<usize, RunCompletionError> {
        Ok(self.recovered)
    }
}

pub fn test_run() -> Run {
    let now = OffsetDateTime::now_utc();
    Run {
        id: RunId::new(),
        instance_id: AgentInstanceId::new(),
        instance_revision_id: AgentInstanceRevisionId::new(),
        release_id: ReleaseId::new(),
        release_agent_id: ReleaseAgentId::new(),
        attachment_id: None,
        kind: RunKind::Normal,
        requires_state: false,
        command_id: CommandId::new(),
        volume_id: None,
        lease_id: None,
        lease_fencing_token: None,
        vm_id: None,
        state: RunState::CleanedUp,
        outcome: Some(RunOutcome::Succeeded),
        exit: None,
        failure: None,
        cancel_requested_at: None,
        created_at: now,
        updated_at: now,
        state_version: 0,
    }
}

#[async_trait]
impl RunCompletionObserver for RecordingCompletion {
    async fn after_cleanup(&self, _run: &Run) -> Result<(), RunCompletionError> {
        lock(&self.log).push("completion");
        Ok(())
    }

    async fn recover(&self) -> Result<usize, RunCompletionError> {
        Ok(0)
    }
}

#[async_trait]
impl RunRuntimeManager for RecordingRuntimeManager {
    async fn prepare(&self, _run: &Run) -> Result<PreparedRunRuntime, RunRuntimeError> {
        lock(&self.log).push("runtime-prepare");
        Ok(PreparedRunRuntime::default())
    }

    async fn destroy(&self, _run_id: RunId) -> Result<(), RunRuntimeError> {
        lock(&self.log).push("runtime-destroy");
        Ok(())
    }

    async fn recover(&self) -> Result<usize, RunRuntimeError> {
        Ok(0)
    }
}

#[async_trait]
impl RuntimeGitWorkspaceManager for RecordingRuntimeGitWorkspaceManager {
    async fn prepare_runtime_git(
        &self,
        _run: &Run,
    ) -> Result<Option<PreparedRuntimeGitWorkspace>, WorkspaceError> {
        lock(&self.log).push("runtime-git-prepare");
        Ok(Some(PreparedRuntimeGitWorkspace {
            id: WorkspaceId::new(),
            mount: VmMount {
                tag: String::from("runtime-git-worktree"),
                host_path: PathBuf::from("/fake/runtime-git"),
                guest_path: PathBuf::from("/workspace/git"),
                read_only: false,
            },
            bridge: vm_trait::RuntimeGitBridge::new(
                Uuid::from_u128(1),
                workspace_domain::RUNTIME_GIT_LOOPBACK_PORT,
            ),
            target_ref: String::from("refs/heads/main"),
            target_commit: "a".repeat(40),
        }))
    }

    async fn abandon_runtime_git(&self, _run_id: RunId) -> Result<(), WorkspaceError> {
        lock(&self.log).push("runtime-git-abandon");
        Ok(())
    }

    async fn recover_runtime_git(&self) -> Result<usize, WorkspaceError> {
        Ok(0)
    }
}

#[async_trait]
impl RunWorkspaceManager for RecordingWorkspaceManager {
    async fn prepare(&self, _run: &Run) -> Result<PreparedWorkspace, WorkspaceError> {
        lock(&self.log).push("prepare");
        Ok(PreparedWorkspace {
            id: Some(WorkspaceId::new()),
            mounts: vec![VmMount {
                tag: String::from("repository-work"),
                host_path: PathBuf::from("/fake/work"),
                guest_path: PathBuf::from("/workspace/work"),
                read_only: false,
            }],
        })
    }

    async fn finalize(
        &self,
        _run: &Run,
        _message: &str,
    ) -> Result<Option<PublishedResult>, WorkspaceError> {
        lock(&self.log).push("finalize");
        Ok(None)
    }

    async fn abandon(&self, _run_id: RunId) -> Result<(), WorkspaceError> {
        lock(&self.log).push("abandon");
        Ok(())
    }

    async fn recover(&self) -> Result<usize, WorkspaceError> {
        Ok(0)
    }
}
