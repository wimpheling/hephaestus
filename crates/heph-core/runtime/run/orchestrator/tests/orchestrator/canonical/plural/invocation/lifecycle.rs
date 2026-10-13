//! The production Core lifecycle through controlled durable/guest ports only.
#[path = "lifecycle/authority.rs"]
mod authority;
#[path = "lifecycle/guest_cwd.rs"]
mod guest_cwd;
#[path = "lifecycle/races.rs"]
mod races;
#[path = "lifecycle/store.rs"]
mod store;
use authority::Authority;
use store::CheckedStore;

use super::{
    fixture::{fixture, wait_finalize},
    ports::{ControlledProvider, NoGit},
};
use crate::canonical::{
    Fixture,
    plural::{Authorizer, Factory, Store},
    provider::Pause,
};
use crate::support::{TransitionPause, lock};
use run_domain::{RunOutcome, RunState};
use run_orchestrator::{RunOrchestrator, RunRepository};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use vm_trait::VmProvider;

struct Controlled {
    fixture: Fixture,
    authority: Arc<Authority>,
    store: Arc<CheckedStore>,
    authorizer: Arc<Authorizer>,
    provider: Arc<ControlledProvider>,
    runner: Arc<RunOrchestrator>,
}
impl Controlled {
    async fn new(count: usize) -> Self {
        Self::with_factory(count, Arc::new(Factory)).await
    }
    async fn with_factory(count: usize, factory: Arc<dyn run_orchestrator::VmSpecFactory>) -> Self {
        let fixture = fixture().await;
        let authority = Arc::new(Authority::new(&fixture));
        let store = Arc::new(CheckedStore {
            inner: Arc::new(Store::new(&fixture, count)),
            authority: authority.clone(),
            partial_attachment: AtomicBool::new(false),
        });
        let authorizer = Arc::new(Authorizer::default());
        let provider = Arc::new(ControlledProvider::new(fixture.provider.clone()));
        let runner = Arc::new(
            RunOrchestrator::new(
                fixture.runs.clone(),
                fixture.volumes.clone(),
                provider.clone(),
                factory,
                32 * 1024 * 1024,
            )
            .with_cleanup_repository(fixture.cleanup.clone(), store.clone())
            .with_launch_authorizer(authorizer.clone())
            .with_volume_preparation()
            .with_authority_manager(authority.clone())
            .with_secret_manager(authority.clone())
            .with_runtime_manager(authority.clone())
            .with_qualified_invocation(fixture.provider.owner_scope().unwrap())
            .with_workspace_manager(Arc::new(NoGit))
            .with_runtime_git_workspace_manager(Arc::new(NoGit)),
        );
        Self {
            fixture,
            authority,
            store,
            authorizer,
            provider,
            runner,
        }
    }
    fn start(
        &self,
    ) -> tokio::task::JoinHandle<Result<run_domain::Run, run_orchestrator::OrchestratorError>> {
        let runner = self.runner.clone();
        let command = self.fixture.command.clone();
        tokio::spawn(async move { runner.start_run(&command).await })
    }
    async fn ready(&self) {
        wait_finalize(&self.fixture).await;
    }
}

#[tokio::test]
async fn invocation_actual_start_and_ack_then_full_attachment_precede_starting_and_running() {
    for count in [0, 2] {
        let controlled = Controlled::new(count).await;
        let pause_start = Pause::new();
        *lock(&controlled.fixture.provider.pause_start) = Some(pause_start.clone());
        let pause_running = TransitionPause {
            entered: Arc::new(tokio::sync::Notify::new()),
            release: Arc::new(tokio::sync::Notify::new()),
        };
        *controlled
            .fixture
            .runs
            .pause_running_transition
            .lock()
            .await = Some(pause_running.clone());
        let running = controlled.start();
        pause_start.entered.notified().await;
        assert_eq!(
            controlled.fixture.runs.run.lock().await.state,
            RunState::Provisioning
        );
        assert!(!controlled.authority.acknowledged.load(Ordering::SeqCst));
        assert_eq!(controlled.fixture.count("current-full-attach"), 0);
        for name in [
            "current-runtime-prepare",
            "current-empty-secret-prepare",
            "current-authority-prepare",
        ] {
            assert_eq!(controlled.fixture.count(name), 1);
        }
        pause_start.release.notify_one();
        pause_running.entered.notified().await;
        assert_eq!(
            controlled.fixture.runs.run.lock().await.state,
            RunState::Starting
        );
        let log = lock(&controlled.fixture.log).clone();
        let index = |name| log.iter().position(|item| *item == name).unwrap();
        assert!(index("start") < index("current-authority-ack"));
        assert!(index("current-authority-ack") < index("current-full-attach"));
        assert_eq!(controlled.fixture.count("current-full-attach"), 1);
        pause_running.release.notify_one();
        controlled.ready().await;
        assert_eq!(
            controlled.fixture.runs.run.lock().await.state,
            RunState::Running
        );
        controlled.provider.finish(0);
        assert_eq!(
            running.await.unwrap().unwrap().outcome,
            Some(RunOutcome::Succeeded)
        );
    }
}

#[tokio::test]
async fn invocation_requires_bootstrap_before_provision_even_without_slots() {
    for count in [0, 2] {
        let controlled = Controlled::new(count).await;
        controlled
            .authority
            .omit_bootstrap
            .store(true, Ordering::SeqCst);
        assert!(
            controlled
                .runner
                .start_run(&controlled.fixture.command)
                .await
                .is_err()
        );
        assert_eq!(controlled.fixture.count("provision"), 0);
        assert_eq!(controlled.fixture.count("start"), 0);
        assert_eq!(controlled.fixture.count("current-full-attach"), 0);
    }
}

#[tokio::test(start_paused = true)]
async fn invocation_wrong_or_absent_actual_ack_never_attaches_or_enters_running() {
    for mode in [1, 2] {
        let controlled = Controlled::new(2).await;
        controlled
            .provider
            .acknowledgement
            .store(mode, Ordering::SeqCst);
        let result = controlled
            .runner
            .start_run(&controlled.fixture.command)
            .await;
        let run = match result {
            Ok(run) => run,
            Err(_) => controlled
                .fixture
                .runs
                .get(controlled.fixture.command.run_id)
                .await
                .unwrap(),
        };
        assert_eq!(run.outcome, Some(RunOutcome::Failed));
        assert_eq!(controlled.fixture.count("current-authority-ack"), 0);
        assert_eq!(controlled.fixture.count("current-full-attach"), 0);
        assert!(run.failure.unwrap().contains("acknowledged a different") || mode == 2);
        assert!(lock(&controlled.fixture.volumes.stale).is_empty());
    }
}

#[tokio::test]
async fn invocation_partial_actual_attachment_retains_fences_without_running() {
    let controlled = Controlled::new(2).await;
    controlled
        .store
        .partial_attachment
        .store(true, Ordering::SeqCst);
    controlled
        .fixture
        .provider
        .fail_cleanup
        .store(true, Ordering::SeqCst);
    assert!(
        controlled
            .runner
            .start_run(&controlled.fixture.command)
            .await
            .is_err()
    );
    let run = controlled.fixture.runs.run.lock().await.clone();
    assert_eq!(run.state, RunState::Provisioning);
    assert!(run.outcome.is_none());
    assert!(controlled.authority.acknowledged.load(Ordering::SeqCst));
    assert_eq!(controlled.fixture.count("current-full-attach"), 1);
    assert_eq!(lock(&controlled.fixture.volumes.stale).len(), 2);
    assert!(
        controlled
            .fixture
            .cleanup
            .state
            .lock()
            .await
            .receipt
            .is_none()
    );
}

#[tokio::test]
async fn invocation_vm_exit_event_and_finalize_cannot_choose_the_actual_wait_result() {
    let controlled = Controlled::new(0).await;
    let running = controlled.start();
    controlled.ready().await;
    controlled
        .provider
        .events
        .send(vm_trait::VmEvent::Exited(vm_trait::VmExit {
            code: Some(0),
            signal: None,
        }))
        .unwrap();
    tokio::task::yield_now().await;
    assert!(!running.is_finished());
    assert!(
        controlled
            .fixture
            .cleanup
            .state
            .lock()
            .await
            .target
            .is_none()
    );
    controlled.provider.finish(7);
    let run = running.await.unwrap().unwrap();
    assert_eq!(run.outcome, Some(RunOutcome::Failed));
    assert_eq!(run.exit.unwrap().code, Some(7));
}
