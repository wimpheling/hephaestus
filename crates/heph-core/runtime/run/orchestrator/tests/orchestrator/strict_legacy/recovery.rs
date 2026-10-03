use super::super::support::lock;
use super::fixture::Fixture;
use run_domain::{LegacyVmPlacementConsumption, RunState};
use run_orchestrator::RunRepository;
use std::sync::atomic::Ordering;
use volume_trait::ScalarLeaseHistory;

#[tokio::test]
async fn vm_null_consumed_open_no_history_remains_available_then_closed_cleanup_uses_plan() {
    let f = Fixture::new(false);
    let orch = f.orchestrator();
    f.runs
        .create_run_with_legacy_placement(&f.command, &f.runs.scope)
        .await
        .unwrap();
    assert!(f.runs.inner.run.lock().await.vm_id.is_none());
    assert_eq!(orch.recover_after_restart().await.unwrap(), 0);
    assert!(lock(&f.log).is_empty());
    f.runs.closed.store(true, Ordering::SeqCst);
    assert_eq!(orch.recover_after_restart().await.unwrap(), 1);
    assert_eq!(f.count("scoped-cleanup"), 1);
    assert_eq!(f.count("completion"), 1);
    assert_eq!(f.runs.inner.run.lock().await.state, RunState::CleanedUp);
}

#[tokio::test]
async fn closed_unconsumed_birth_uses_cleanup_only_and_can_never_start() {
    let f = Fixture::new(false);
    f.runs
        .create_run_with_legacy_placement(&f.command, &f.runs.scope)
        .await
        .unwrap();
    let p = f.runs.plan.lock().await.clone().unwrap();
    *f.runs.plan.lock().await = Some(
        run_domain::LegacyVmPlacement::new(
            f.command.clone(),
            p.project(),
            *p.contract_hash(),
            p.scope().clone(),
            p.vm_id().clone(),
            p.producer(),
            None,
        )
        .unwrap(),
    );
    let orch = f.orchestrator();
    assert_eq!(orch.recover_after_restart().await.unwrap(), 0);
    f.runs.closed.store(true, Ordering::SeqCst);
    assert_eq!(orch.recover_after_restart().await.unwrap(), 1);
    assert_eq!(
        f.runs.plan.lock().await.as_ref().unwrap().consumption(),
        Some(LegacyVmPlacementConsumption::CleanupOnly)
    );
    let physical = f.count("scoped-cleanup");
    orch.start_run(&f.command).await.unwrap();
    assert_eq!(f.count("start"), 0);
    assert_eq!(f.count("scoped-cleanup"), physical);
}

#[tokio::test]
async fn restart_releases_exact_nonexpired_held_tuple_only_after_scoped_confirmation() {
    let f = Fixture::new(true);
    let orch = f.orchestrator();
    f.runs
        .create_run_with_legacy_placement(&f.command, &f.runs.scope)
        .await
        .unwrap();
    let lease = f.volumes.set_held(f.command.run_id).await;
    f.runs.inner.run.lock().await.state = RunState::Provisioning;
    assert!(lease.expires_at > time::OffsetDateTime::now_utc());
    assert_eq!(orch.recover_stale_leases().await.unwrap(), 0);
    assert_eq!(orch.recover_after_restart().await.unwrap(), 1);
    assert_eq!(f.count("scoped-cleanup"), 1);
    assert_eq!(f.count("release"), 1);
    assert_eq!(f.count("recover-finish"), 0);
    assert_eq!(f.count("authority-recover"), 0);
}

#[tokio::test]
async fn persisted_recovery_marker_uses_only_recovery_release_not_expiry_inference() {
    let f = Fixture::new(true);
    let orch = f.orchestrator();
    f.runs
        .create_run_with_legacy_placement(&f.command, &f.runs.scope)
        .await
        .unwrap();
    let lease = f.volumes.set_held(f.command.run_id).await;
    *f.volumes.history.lock().await = ScalarLeaseHistory::HeldRecovering(lease);
    f.runs.inner.run.lock().await.state = RunState::Provisioning;
    assert_eq!(orch.recover_after_restart().await.unwrap(), 1);
    assert_eq!(f.count("recover-finish"), 1);
    assert_eq!(f.count("release"), 0);
}

#[tokio::test]
async fn released_original_tuple_replay_and_terminal_callback_do_not_repeat_physical_io() {
    let f = Fixture::new(true);
    let orch = f.orchestrator();
    f.runs
        .create_run_with_legacy_placement(&f.command, &f.runs.scope)
        .await
        .unwrap();
    let original = f.volumes.set_held(f.command.run_id).await;
    *f.volumes.history.lock().await = ScalarLeaseHistory::Released(original.clone());
    f.runs.closed.store(true, Ordering::SeqCst);
    f.runs.inner.run.lock().await.state = RunState::CleaningUp;
    assert_eq!(orch.recover_after_restart().await.unwrap(), 1);
    assert_eq!(
        *f.volumes.history.lock().await,
        ScalarLeaseHistory::Released(original)
    );
    assert_eq!(f.count("release"), 0);
    let physical = f.count("scoped-cleanup");
    orch.start_run(&f.command).await.unwrap();
    assert_eq!(f.count("scoped-cleanup"), physical);
    assert_eq!(f.count("completion"), 2);
}

#[tokio::test]
async fn foreign_history_fence_is_held_without_scalar_release_or_callback() {
    let f = Fixture::new(true);
    let orch = f.orchestrator();
    f.runs
        .create_run_with_legacy_placement(&f.command, &f.runs.scope)
        .await
        .unwrap();
    let mut foreign = f.volumes.set_held(f.command.run_id).await;
    foreign.fencing_token += 1;
    *f.volumes.history.lock().await = ScalarLeaseHistory::Held(foreign.clone());
    assert!(orch.recover_after_restart().await.is_err());
    assert_eq!(
        *f.volumes.history.lock().await,
        ScalarLeaseHistory::Held(foreign)
    );
    assert_eq!(f.count("release"), 0);
    assert_eq!(f.count("completion"), 0);
}

#[tokio::test]
async fn unknown_held_inventory_is_not_host_filtered_to_absence_or_global_sweep() {
    let f = Fixture::new(false);
    f.runs.held_inventory.store(true, Ordering::SeqCst);
    let orch = f.orchestrator();
    assert_eq!(orch.recover_stale_leases().await.unwrap(), 0);
    assert_eq!(orch.recover_after_restart().await.unwrap(), 0);
    assert!(lock(&f.log).is_empty());
}
#[tokio::test]
async fn contradictory_closed_inventory_rejects_before_any_physical_action() {
    let f = Fixture::new(false);
    f.runs
        .create_run_with_legacy_placement(&f.command, &f.runs.scope)
        .await
        .unwrap();
    f.runs.bad_inventory.store(true, Ordering::SeqCst);
    assert!(f.orchestrator().recover_after_restart().await.is_err());
    assert!(lock(&f.log).is_empty());
}
