use runtime_types::RunId;
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::sync::Mutex;
use vm_trait::{VmInstance, VmProvider};
use volume_trait::VolumeStore;
use workspace_domain::{DisabledWorkspaceManager, RunWorkspaceManager, RuntimeGitWorkspaceManager};

use super::{
    authority::{
        DisabledRunAuthorityManager, DisabledRunLaunchAuthorizer,
        DisabledRuntimeGitWorkspaceManager, RunAuthorityManager,
    },
    runtime::{
        DisabledRunResourceObserver, RunLaunchAuthorizer, RunResourceObserver, RunRuntimeManager,
    },
    secrets::{
        DisabledRunCompletionObserver, DisabledRunRuntimeManager, DisabledRunSecretManager,
        RunCompletionObserver, RunSecretManager,
    },
};
use crate::{RunRepository, VmSpecFactory};

/// Durable coordinator for run, volume, and VM lifecycles.
pub struct RunOrchestrator {
    pub(crate) repository: Arc<dyn RunRepository>,
    pub(crate) volumes: Arc<dyn VolumeStore>,
    pub(crate) provider: Arc<dyn VmProvider>,
    pub(crate) spec_factory: Arc<dyn VmSpecFactory>,
    pub(crate) workspaces: Arc<dyn RunWorkspaceManager>,
    pub(crate) runtime_git_workspace: Arc<dyn RuntimeGitWorkspaceManager>,
    pub(crate) runtimes: Arc<dyn RunRuntimeManager>,
    pub(crate) launch_authorizer: Arc<dyn RunLaunchAuthorizer>,
    pub(crate) resource_observer: Arc<dyn RunResourceObserver>,
    pub(crate) authority: Arc<dyn RunAuthorityManager>,
    pub(crate) secrets: Arc<dyn RunSecretManager>,
    pub(crate) completion: Arc<dyn RunCompletionObserver>,
    pub(crate) active: Mutex<HashMap<RunId, Arc<dyn VmInstance>>>,
    pub(crate) instance_state_capacity_bytes: u64,
    pub(crate) cancellation_timeout: Duration,
}

impl RunOrchestrator {
    /// Creates an orchestrator from its durable and provider-neutral
    /// boundaries.
    #[must_use]
    pub fn new(
        repository: Arc<dyn RunRepository>,
        volumes: Arc<dyn VolumeStore>,
        provider: Arc<dyn VmProvider>,
        spec_factory: Arc<dyn VmSpecFactory>,
        instance_state_capacity_bytes: u64,
    ) -> Self {
        Self {
            repository,
            volumes,
            provider,
            spec_factory,
            workspaces: Arc::new(DisabledWorkspaceManager),
            runtime_git_workspace: Arc::new(DisabledRuntimeGitWorkspaceManager),
            runtimes: Arc::new(DisabledRunRuntimeManager),
            launch_authorizer: Arc::new(DisabledRunLaunchAuthorizer),
            resource_observer: Arc::new(DisabledRunResourceObserver),
            authority: Arc::new(DisabledRunAuthorityManager),
            secrets: Arc::new(DisabledRunSecretManager),
            completion: Arc::new(DisabledRunCompletionObserver),
            active: Mutex::new(HashMap::new()),
            instance_state_capacity_bytes,
            cancellation_timeout: Duration::from_secs(10),
        }
    }

    /// Installs the trusted repository workspace and result lifecycle manager.
    #[must_use]
    pub fn with_workspace_manager(mut self, workspaces: Arc<dyn RunWorkspaceManager>) -> Self {
        self.workspaces = workspaces;
        self
    }

    /// Installs the isolated runtime-Git worktree lifecycle manager.
    #[must_use]
    pub fn with_runtime_git_workspace_manager(
        mut self,
        runtime_git_workspace: Arc<dyn RuntimeGitWorkspaceManager>,
    ) -> Self {
        self.runtime_git_workspace = runtime_git_workspace;
        self
    }

    /// Installs exact release-artifact and host-context materialization.
    #[must_use]
    pub fn with_runtime_manager(mut self, runtimes: Arc<dyn RunRuntimeManager>) -> Self {
        self.runtimes = runtimes;
        self
    }

    /// Installs the live authorization boundary used before artifact
    /// materialization and immediately before VM provisioning.
    #[must_use]
    pub fn with_launch_authorizer(
        mut self,
        launch_authorizer: Arc<dyn RunLaunchAuthorizer>,
    ) -> Self {
        self.launch_authorizer = launch_authorizer;
        self
    }

    /// Installs an idempotent pre-provisioning observer for exact run
    /// resource evidence.
    #[must_use]
    pub fn with_resource_observer(mut self, observer: Arc<dyn RunResourceObserver>) -> Self {
        self.resource_observer = observer;
        self
    }

    /// Installs generic runtime-authority issuance and handoff lifecycle
    /// coordination.
    #[must_use]
    pub fn with_authority_manager(mut self, authority: Arc<dyn RunAuthorityManager>) -> Self {
        self.authority = authority;
        self
    }

    /// Installs live secret dispatch and ephemeral mount materialization.
    #[must_use]
    pub fn with_secret_manager(mut self, secrets: Arc<dyn RunSecretManager>) -> Self {
        self.secrets = secrets;
        self
    }

    /// Installs idempotent post-cleanup domain result processing.
    #[must_use]
    pub fn with_completion_observer(mut self, completion: Arc<dyn RunCompletionObserver>) -> Self {
        self.completion = completion;
        self
    }
}
