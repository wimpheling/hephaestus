//! Core-only Invocation tests use controlled admissions and guests, not SQL/VM proof.
#[path = "invocation/fixture.rs"]
mod fixture;
#[path = "invocation/ports.rs"]
mod ports;

use super::{Authorizer, Factory};
use crate::support::lock;
use fixture::{configured, controlled, fixture, wait_finalize, with_authorizer};
use ports::{ControlledProvider, GitOverrideFactory};
use run_domain::{CancelRun, RunOutcome, RunState};
use run_orchestrator::{OrchestratorError, RunRepository};
use runtime_types::CommandId;
use std::{
    sync::{Arc, atomic::Ordering},
    time::Duration,
};
use vm_trait::{VmProvider, VmProviderOwnerScope};

#[tokio::test]
async fn invocation_success_and_replay_have_no_git_for_zero_two_and_thirty_two_leases() {
    for count in [0, 2, 32] {
        let fixture = fixture().await;
        let runner = configured(&fixture, count, fixture.provider.clone(), Arc::new(Factory));
        let run = runner.start_run(&fixture.command).await.unwrap();
        assert_eq!(run.state, RunState::CleanedUp);
        assert_eq!(run.outcome, Some(RunOutcome::Succeeded));
        let spec = fixture.provider.inner.spec().unwrap();
        assert_eq!(spec.disks.len(), count);
        assert_eq!(spec.guest_volumes.len(), count);
        assert!(spec.mounts.is_empty());
        assert!(spec.command.working_dir.is_none());
        assert!(spec.runtime_git_bridge.is_none());
        assert!(lock(&fixture.volumes.stale).is_empty());
        let counts = (fixture.count("provision"), fixture.count("finish-all"));
        assert_eq!(runner.start_run(&fixture.command).await.unwrap(), run);
        assert_eq!(
            (fixture.count("provision"), fixture.count("finish-all")),
            counts
        );
    }
}

#[tokio::test]
async fn invocation_finalize_cannot_stop_guest_or_override_actual_failed_exit() {
    let fixture = fixture().await;
    let (runner, provider) = controlled(&fixture, 2);
    let running = tokio::spawn({
        let runner = runner.clone();
        let command = fixture.command.clone();
        async move { runner.start_run(&command).await }
    });
    wait_finalize(&fixture).await;
    assert!(
        !running.is_finished(),
        "Finalize cannot complete Invocation"
    );
    assert_eq!(fixture.count("stop"), 0);
    provider.finish(1);
    let run = running.await.unwrap().unwrap();
    assert_eq!(run.outcome, Some(RunOutcome::Failed));
    assert_eq!(run.exit.unwrap().code, Some(1));
    assert_eq!(fixture.count("stop"), 0);
    assert!(lock(&fixture.volumes.stale).is_empty());
}

#[tokio::test]
async fn invocation_waits_for_actual_successful_exit_after_finalize() {
    let fixture = fixture().await;
    let (runner, provider) = controlled(&fixture, 0);
    let running = tokio::spawn({
        let runner = runner.clone();
        let command = fixture.command.clone();
        async move { runner.start_run(&command).await }
    });
    wait_finalize(&fixture).await;
    assert!(!running.is_finished());
    assert_eq!(fixture.count("stop"), 0);
    provider.finish(0);
    assert_eq!(
        running.await.unwrap().unwrap().outcome,
        Some(RunOutcome::Succeeded)
    );
}

#[tokio::test(start_paused = true)]
async fn invocation_current_cancel_wins_exit_and_monitor_races_without_git() {
    for (monitor_wins, unknown_cleanup) in [(false, false), (true, false), (true, true)] {
        let fixture = fixture().await;
        let (runner, provider) = controlled(&fixture, 2);
        provider.stop_exits.store(!monitor_wins, Ordering::SeqCst);
        fixture
            .provider
            .fail_cleanup
            .store(unknown_cleanup, Ordering::SeqCst);
        let running = tokio::spawn({
            let runner = runner.clone();
            let command = fixture.command.clone();
            async move { runner.start_run(&command).await }
        });
        wait_finalize(&fixture).await;
        runner
            .cancel_run(&CancelRun {
                command_id: CommandId::new(),
                run_id: fixture.command.run_id,
                reason: "current cancellation".into(),
            })
            .await
            .unwrap();
        if monitor_wins {
            // Stop did not yield an exit: the monitor must confirm scoped
            // destruction before choosing Cancelled from the fresh snapshot.
            tokio::time::advance(Duration::from_secs(1)).await;
            assert!(running.await.unwrap().is_err());
        } else {
            assert_eq!(
                running.await.unwrap().unwrap().outcome,
                Some(RunOutcome::Cancelled)
            );
        }
        let run = fixture.runs.get(fixture.command.run_id).await.unwrap();
        assert_eq!(fixture.count("stop"), 1);
        assert!(fixture.count("scoped-cleanup") >= 1);
        if unknown_cleanup {
            assert_eq!(run.state, RunState::Running);
            assert!(run.outcome.is_none());
            assert!(fixture.cleanup.state.lock().await.receipt.is_none());
            assert_eq!(lock(&fixture.volumes.stale).len(), 2);
        } else {
            assert_eq!(run.state, RunState::CleanedUp);
            assert_eq!(run.outcome, Some(RunOutcome::Cancelled));
            assert!(fixture.cleanup.state.lock().await.receipt.is_some());
            assert!(lock(&fixture.volumes.stale).is_empty());
        }
    }
}

#[tokio::test]
async fn invocation_missing_or_wrong_controlled_admission_refuses_before_provider_io() {
    for missing in [false, true] {
        let fixture = fixture().await;
        if missing {
            fixture
                .runs
                .invocation_admitted
                .store(false, Ordering::SeqCst);
        } else {
            fixture.runs.run.lock().await.instance_revision_id =
                runtime_types::AgentInstanceRevisionId::new();
        }
        let before = fixture.runs.get(fixture.command.run_id).await.unwrap();
        let runner = configured(&fixture, 2, fixture.provider.clone(), Arc::new(Factory));
        assert!(runner.start_run(&fixture.command).await.is_err());
        assert_eq!(
            fixture.runs.get(fixture.command.run_id).await.unwrap(),
            before
        );
        assert_eq!(fixture.count("provision"), 0);
        assert!(lock(&fixture.volumes.stale).is_empty());
        assert!(fixture.cleanup.state.lock().await.target.is_none());
    }
    let fixture = fixture().await;
    let runner = configured(
        &fixture,
        2,
        fixture.provider.clone(),
        Arc::new(GitOverrideFactory),
    );
    assert!(runner.start_run(&fixture.command).await.is_err());
    assert_eq!(fixture.count("provision"), 0);
    assert!(lock(&fixture.volumes.stale).is_empty());
}

#[tokio::test]
async fn invocation_wrong_actual_owner_and_invalid_scalar_config_refuse_before_admission() {
    let fixture = fixture().await;
    let expected = fixture.provider.owner_scope().unwrap();
    let invalid = fixture.orchestrator().with_qualified_invocation(expected);
    assert!(matches!(
        invalid.start_run(&fixture.command).await,
        Err(OrchestratorError::InvocationUnsupported)
    ));
    let ownerless = configured(
        &fixture,
        0,
        Arc::new(crate::support::AutoExitProvider::new(fixture.log.clone())),
        Arc::new(Factory),
    );
    assert!(matches!(
        ownerless.start_run(&fixture.command).await,
        Err(OrchestratorError::InvocationUnsupported)
    ));
    let runner = configured(&fixture, 0, fixture.provider.clone(), Arc::new(Factory));
    *lock(&fixture.provider.scope) =
        VmProviderOwnerScope::new("other-owner".into(), "test".into()).unwrap();
    assert!(matches!(
        runner.start_run(&fixture.command).await,
        Err(OrchestratorError::InvocationUnsupported)
    ));
    assert_eq!(fixture.count("bind-planned"), 0);
    assert_eq!(fixture.count("provision"), 0);
    assert!(fixture.cleanup.state.lock().await.target.is_none());
}

#[tokio::test(start_paused = true)]
async fn invocation_failed_cleanup_holds_fences_and_source_failure_stays_failed() {
    let fixture = fixture().await;
    fixture.provider.fail_cleanup.store(true, Ordering::SeqCst);
    let runner = configured(&fixture, 2, fixture.provider.clone(), Arc::new(Factory));
    assert!(runner.start_run(&fixture.command).await.is_err());
    assert_eq!(lock(&fixture.volumes.stale).len(), 2);
    assert!(fixture.cleanup.state.lock().await.receipt.is_none());
    assert_eq!(fixture.count("finish-all"), 0);

    let denied = fixture::fixture().await;
    let provider = Arc::new(ControlledProvider::new(denied.provider.clone()));
    let authorizer = Arc::new(Authorizer::default());
    let runner = Arc::new(with_authorizer(
        &denied,
        2,
        provider,
        Arc::new(Factory),
        authorizer.clone(),
    ));
    let running = tokio::spawn({
        let runner = runner.clone();
        let command = denied.command.clone();
        async move { runner.start_run(&command).await }
    });
    wait_finalize(&denied).await;
    authorizer.denied.store(true, Ordering::SeqCst);
    tokio::time::advance(Duration::from_secs(1)).await;
    assert!(running.await.unwrap().is_err());
    assert_eq!(
        denied.runs.run.lock().await.outcome,
        Some(RunOutcome::Failed)
    );
    assert!(lock(&denied.volumes.stale).is_empty());
}

#[tokio::test]
async fn invocation_restart_cleanup_uses_exact_run_without_git_sweeps() {
    let fixture = fixture().await;
    fixture.runs.run.lock().await.state = RunState::Running;
    let runner = configured(&fixture, 0, fixture.provider.clone(), Arc::new(Factory));
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), runner.recover_after_restart())
            .await
            .unwrap()
            .unwrap(),
        1
    );
    assert_eq!(fixture.runs.run.lock().await.state, RunState::CleanedUp);
    assert_eq!(fixture.count("scoped-cleanup"), 1);
}
