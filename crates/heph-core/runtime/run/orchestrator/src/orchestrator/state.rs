use runtime_types::RunId;
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::sync::Mutex;
use vm_trait::{VmInstance, VmProvider};
use volume_trait::{RunVolumeStore, VolumeStore};
use workspace_domain::{DisabledWorkspaceManager, RunWorkspaceManager, RuntimeGitWorkspaceManager};

use super::{
    authority::{
        DisabledRunAuthorityManager, DisabledRunLaunchAuthorizer,
        DisabledRuntimeGitWorkspaceManager, RunAuthorityManager,
    },
    canonical_cleanup::CanonicalCleanup,
    operation_guards::RunOperationGuards,
    runtime::{
        DisabledRunResourceObserver, RunLaunchAuthorizer, RunResourceObserver, RunRuntimeManager,
    },
    secrets::{
        DisabledRunCompletionObserver, DisabledRunRuntimeManager, DisabledRunSecretManager,
        RunCompletionObserver, RunSecretManager,
    },
};
use crate::{RunCleanupRepository, RunRepository, VmSpecFactory};

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
    pub(crate) canonical_cleanup: Option<CanonicalCleanup>,
    pub(crate) cleanup_timeout: Duration,
    pub(crate) plural_volumes: bool,
    pub(crate) operation_guards: Arc<RunOperationGuards>,
    pub(crate) legacy_scope: Option<run_domain::LegacyVmPlacementScope>,
    pub(crate) legacy_plans: Mutex<HashMap<RunId, run_domain::LegacyVmPlacement>>,
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
            canonical_cleanup: None,
            cleanup_timeout: Duration::from_secs(30),
            plural_volumes: false,
            operation_guards: Arc::new(RunOperationGuards::default()),
            legacy_scope: None,
            legacy_plans: Mutex::new(HashMap::new()),
        }
    }

    /// Opts into prospective managed Legacy placement and scoped scalar cleanup.
    ///
    /// Requires the exact provider owner and supported placement/history ports.
    /// This is mutually exclusive with canonical cleanup and named preparation.
    #[must_use]
    pub fn with_legacy_vm_placement(mut self, scope: run_domain::LegacyVmPlacementScope) -> Self {
        self.legacy_scope = Some(scope);
        self
    }
    /// Shares admission and physical IO guards for one managed provider owner.
    ///
    /// All canonical and Legacy orchestrators for the same provider clone family
    /// must share this registry. No guard is held across ordinary guest execution.
    #[must_use]
    pub fn with_operation_guards(mut self, guards: Arc<RunOperationGuards>) -> Self {
        self.operation_guards = guards;
        self
    }
    /// Opts into durable complete-set cleanup and exact provider ownership.
    ///
    /// The worker repository closes acquisition and persists trusted scoped
    /// observations. The exact-run volume port supplies global held-lease
    /// evidence for grandfathered already-cleaned historical runs. Providers
    /// without durable ownership fail before preparation or acquisition.
    /// This does not enable named-volume dispatch.
    #[must_use]
    pub fn with_cleanup_repository(
        mut self,
        repository: Arc<dyn RunCleanupRepository>,
        volumes: Arc<dyn RunVolumeStore>,
    ) -> Self {
        self.canonical_cleanup = Some(CanonicalCleanup {
            repository,
            volumes,
        });
        self
    }

    /// Opts into complete persisted selection preparation and live monitoring.
    ///
    /// Requires the canonical cleanup repository, an explicitly implemented
    /// plural spec factory and a trusted live caller/source authorizer. The
    /// default is disabled. This alone does not enable database named dispatch.
    #[must_use]
    pub const fn with_volume_preparation(mut self) -> Self {
        self.plural_volumes = true;
        self
    }

    /// Sets the combined physical destruction and scoped confirmation deadline.
    ///
    /// The default and maximum are 30 seconds. Timeout keeps every fence and
    /// any active handle, with no cleanup receipt. This controls only the
    /// opt-in canonical or strict Legacy cleanup path.
    #[must_use]
    pub fn with_cleanup_timeout(mut self, timeout: Duration) -> Self {
        self.cleanup_timeout = timeout.min(Duration::from_secs(30));
        self
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
