use run_orchestrator::RunOrchestrator;
use std::sync::Arc;
use vm_trait::VmProvider;

use super::super::{
    fixture::{fixture, wait_finalize},
    ports::{ControlledProvider, NoGit},
};
use super::{ports::Ports, store::CheckedStore};
use crate::canonical::{Fixture, plural::Factory};

pub struct Controlled {
    pub fixture: Fixture,
    pub ports: Arc<Ports>,
    pub store: Arc<CheckedStore>,
    pub provider: Arc<ControlledProvider>,
}
impl Controlled {
    pub async fn new(count: usize) -> Self {
        Self::from_fixture(fixture().await, count)
    }
    pub fn from_fixture(fixture: Fixture, count: usize) -> Self {
        let ports = Arc::new(Ports::new(&fixture));
        let store = Arc::new(CheckedStore::new(&fixture, count));
        let provider = Arc::new(ControlledProvider::new(fixture.provider.clone()));
        Self {
            fixture,
            ports,
            store,
            provider,
        }
    }
    pub fn runner(&self, configured: bool, launch: bool) -> RunOrchestrator {
        let mut runner = RunOrchestrator::new(
            self.fixture.runs.clone(),
            self.fixture.volumes.clone(),
            self.provider.clone(),
            Arc::new(Factory),
            32 * 1024 * 1024,
        )
        .with_cleanup_repository(self.fixture.cleanup.clone(), self.store.clone())
        .with_volume_preparation()
        .with_authority_manager(self.ports.clone())
        .with_secret_manager(self.ports.clone())
        .with_runtime_manager(self.ports.clone());
        if configured {
            runner = runner.with_qualified_invocation(self.fixture.provider.owner_scope().unwrap());
        }
        if launch {
            runner = runner.with_launch_authorizer(self.ports.clone());
        }
        // Invocation keeps strict Git tripwires even when configuration is absent.
        // Ordinary refusals retain the existing disabled managers for cleanup.
        if self.fixture.command.kind == run_domain::RunKind::Invocation {
            runner = runner
                .with_workspace_manager(Arc::new(NoGit))
                .with_runtime_git_workspace_manager(Arc::new(NoGit));
        }
        runner
    }
    pub async fn complete(&self) {
        let runner = Arc::new(self.runner(true, true));
        let command = self.fixture.command.clone();
        let task = tokio::spawn(async move { runner.start_run(&command).await });
        wait_finalize(&self.fixture).await;
        self.provider.finish(0);
        task.await.unwrap().unwrap();
    }
    pub fn no_acquisition_or_materialization(&self) {
        for name in ["first-acquire", "first-runtime", "provision", "start"] {
            assert_eq!(self.fixture.count(name), 0, "{name}");
        }
        assert!(crate::support::lock(&self.fixture.volumes.stale).is_empty());
    }
}
