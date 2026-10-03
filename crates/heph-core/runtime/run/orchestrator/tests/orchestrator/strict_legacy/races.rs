use super::super::{canonical::provider::Pause, support::lock};
use super::fixture::Fixture;
use run_orchestrator::{OrchestratorError, RunRepository};
use std::sync::Arc;
use std::sync::atomic::Ordering;

#[tokio::test]
async fn duplicate_start_does_not_prepare_or_clean_another_live_start() {
    let f = Fixture::new(false);
    let pause = Pause::new();
    *lock(&f.provider.pause_provision) = Some(pause.clone());
    let orch = Arc::new(f.orchestrator());
    let first = {
        let orch = orch.clone();
        let c = f.command.clone();
        tokio::spawn(async move { orch.start_run(&c).await })
    };
    pause.entered.notified().await;
    let effects = lock(&f.log).len();
    assert!(matches!(
        f.orchestrator().start_run(&f.command).await,
        Err(OrchestratorError::RunInProgress(_))
    ));
    assert_eq!(lock(&f.log).len(), effects);
    assert_eq!(f.count("scoped-cleanup"), 0);
    pause.release.notify_one();
    first.await.unwrap().unwrap();
    assert_eq!(f.count("provision"), 1);
    assert_eq!(f.count("completion"), 1);
}

#[tokio::test]
async fn close_during_paused_provider_entry_blocks_later_start_and_waits_quiescence() {
    let f = Fixture::new(false);
    let pause = Pause::new();
    *lock(&f.provider.pause_provision) = Some(pause.clone());
    let orch = Arc::new(f.orchestrator());
    let first = {
        let orch = orch.clone();
        let c = f.command.clone();
        tokio::spawn(async move { orch.start_run(&c).await })
    };
    pause.entered.notified().await;
    f.runs
        .close_legacy_vm_acquisition(f.command.run_id, &f.runs.scope)
        .await
        .unwrap();
    assert_eq!(f.count("scoped-cleanup"), 0);
    pause.release.notify_one();
    assert!(first.await.unwrap().is_err());
    assert_eq!(f.count("start"), 0);
    assert!(f.runs.closed.load(Ordering::SeqCst));
    assert_eq!(f.count("completion"), 1);
}

#[tokio::test]
async fn shared_registry_allows_parallel_other_runs_while_first_vm_provision_waits() {
    let a = Fixture::new(false);
    let mut b = Fixture::new(false);
    b.guards = a.guards.clone();
    b.provider = a.provider.clone();
    b.runs = Arc::new(super::repository::Repository::new(
        &b.command,
        a.runs.scope.clone(),
    ));
    b.volumes = Arc::new(super::volumes::Volumes {
        inner: super::super::support::MemoryVolumeStore::new(b.command.instance_id, b.log.clone()),
        runs: b.runs.clone(),
        history: tokio::sync::Mutex::new(volume_trait::ScalarLeaseHistory::NoHistory),
        fail_after_acquire: std::sync::atomic::AtomicBool::new(false),
    });
    let pause = Pause::new();
    *lock(&a.provider.pause_provision) = Some(pause.clone());
    let orch = Arc::new(a.orchestrator());
    let first = {
        let orch = orch.clone();
        let c = a.command.clone();
        tokio::spawn(async move { orch.start_run(&c).await })
    };
    pause.entered.notified().await;
    // The first call retained its exact pause; another VM enters the same family.
    *lock(&a.provider.pause_provision) = None;
    b.orchestrator().start_run(&b.command).await.unwrap();
    assert_eq!(a.count("start"), 1);
    assert!(!first.is_finished());
    pause.release.notify_one();
    first.await.unwrap().unwrap();
}
