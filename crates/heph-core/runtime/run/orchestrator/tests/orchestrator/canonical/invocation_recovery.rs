//! Controlled core recovery only; `PostgreSQL` qualification is tested separately.
#[path = "invocation_inventory.rs"]
pub(super) mod inventory;

use super::super::support::{CountingCompletion, TestSpecFactory, lock};
use super::{Fixture, volumes::GlobalVolumes};
use async_trait::async_trait;
use inventory::Inventory;
use run_domain::{Run, RunKind, RunOutcome, RunState};
use run_orchestrator::{PreparedRunRuntime, RunOrchestrator, RunRuntimeError, RunRuntimeManager};
use runtime_types::RunId;
use std::{
    path::PathBuf,
    sync::{Arc, Mutex, atomic::Ordering},
};
use time::OffsetDateTime;
use vm_trait::VmProvider;

fn inventory(fixture: &Fixture, ids: Vec<RunId>) -> Arc<Inventory> {
    Arc::new(Inventory {
        inner: fixture.cleanup.clone(),
        scope: fixture.provider.owner_scope().unwrap(),
        ids,
        cutoffs: Mutex::new(Vec::new()),
    })
}
fn qualified(fixture: &Fixture, inventory: Arc<Inventory>) -> RunOrchestrator {
    super::invocation_preparation_repository::Repository::new(fixture, false)
        .orchestrator(fixture)
        .with_cleanup_repository(inventory, Arc::new(GlobalVolumes(fixture.volumes.clone())))
        .with_qualified_invocation(fixture.provider.owner_scope().unwrap())
}

struct SweepSentinel(PathBuf);
#[async_trait]
impl RunRuntimeManager for SweepSentinel {
    async fn prepare(&self, _: &Run) -> Result<PreparedRunRuntime, RunRuntimeError> {
        Ok(PreparedRunRuntime::default())
    }
    async fn destroy(&self, _: RunId) -> Result<(), RunRuntimeError> {
        Ok(())
    }
    async fn recover(&self) -> Result<usize, RunRuntimeError> {
        std::fs::remove_file(&self.0).unwrap();
        panic!("qualified Invocation recovery must never sweep shared Legacy resources")
    }
}

#[tokio::test]
async fn empty_qualified_inventory_preserves_shared_sentinel_and_skips_global_completion() {
    let fixture = Fixture::new();
    // This Legacy-shaped row/lease must not be obtained through scalar inventory.
    fixture.prepare_recovery().await;
    fixture.add_leases(1);
    let sentinel = std::env::temp_dir().join(format!(
        "heph-invocation-recovery-{}.sentinel",
        fixture.command.run_id
    ));
    std::fs::write(&sentinel, b"shared Legacy allocation").unwrap();
    let inventory = inventory(&fixture, vec![]);
    let orchestrator = qualified(&fixture, inventory.clone())
        .with_runtime_manager(Arc::new(SweepSentinel(sentinel.clone())))
        .with_completion_observer(Arc::new(CountingCompletion::new(99)));
    assert_eq!(orchestrator.recover_after_restart().await.unwrap(), 0);
    assert_eq!(orchestrator.recover_stale_leases().await.unwrap(), 0);
    drop(orchestrator);
    assert_eq!(
        std::fs::read(&sentinel).unwrap(),
        b"shared Legacy allocation"
    );
    assert_eq!(lock(&fixture.volumes.stale).len(), 1);
    assert_eq!(fixture.count("scoped-cleanup"), 0);
    let cutoffs = lock(&inventory.cutoffs).clone();
    drop(inventory);
    assert_eq!(cutoffs.len(), 2);
    assert!(cutoffs[0].is_none() && cutoffs[1].is_some());
    std::fs::remove_file(sentinel).unwrap();
}

#[tokio::test]
async fn qualified_plural_failure_preserves_both_fences_then_exact_retry_finishes_once() {
    let fixture = Fixture::new();
    fixture.prepare_recovery().await;
    {
        let mut run = fixture.runs.run.lock().await;
        run.kind = RunKind::Invocation;
        run.attachment_id = None;
    }
    fixture.add_leases(2);
    let inventory = inventory(&fixture, vec![fixture.command.run_id]);
    let orchestrator = qualified(&fixture, inventory.clone());
    fixture.provider.fail_cleanup.store(true, Ordering::SeqCst);
    assert!(orchestrator.recover_stale_leases().await.is_err());
    let target = fixture.cleanup.state.lock().await.target.clone().unwrap();
    assert_eq!(target.leases().len(), 2);
    assert_eq!(lock(&fixture.volumes.stale).len(), 2);
    assert!(fixture.cleanup.state.lock().await.receipt.is_none());
    assert_eq!(fixture.count("finish-all"), 0);
    fixture.provider.fail_cleanup.store(false, Ordering::SeqCst);
    assert_eq!(orchestrator.recover_after_restart().await.unwrap(), 1);
    drop(orchestrator);
    assert_eq!(
        fixture.cleanup.state.lock().await.target.as_ref(),
        Some(&target)
    );
    assert!(lock(&fixture.volumes.stale).is_empty());
    assert_eq!(fixture.count("finish-all"), 1);
    assert_eq!(fixture.count("recover-begin"), 0);
    assert_eq!(fixture.count("recover-finish"), 0);
    assert!(lock(&inventory.cutoffs)[0].is_some());
    drop(inventory);
}

#[tokio::test]
async fn qualified_cancelled_queued_recovery_preserves_cancelled_outcome_without_failure() {
    let fixture = Fixture::new();
    fixture.prepare_recovery().await;
    {
        let mut run = fixture.runs.run.lock().await;
        run.kind = RunKind::Invocation;
        run.attachment_id = None;
        run.state = RunState::Queued;
        run.cancel_requested_at = Some(OffsetDateTime::now_utc());
    }
    // Controlled protected preparation runs before strict queued hydration.
    let repository = super::invocation_preparation_repository::Repository::new(&fixture, true);
    let orchestrator = repository.orchestrator(&fixture).with_cleanup_repository(
        inventory(&fixture, vec![fixture.command.run_id]),
        Arc::new(GlobalVolumes(fixture.volumes.clone())),
    );
    assert_eq!(orchestrator.recover_after_restart().await.unwrap(), 1);
    drop(orchestrator);
    let run = fixture.runs.run.lock().await.clone();
    assert_eq!(run.state, RunState::CleanedUp);
    assert_eq!(run.outcome, Some(RunOutcome::Cancelled));
    assert!(run.failure.is_none() && run.exit.is_none());
}

#[tokio::test]
async fn duplicate_ids_and_unsupported_inventory_fail_before_cleanup() {
    let fixture = Fixture::new();
    fixture.prepare_recovery().await;
    let duplicate = qualified(
        &fixture,
        inventory(&fixture, vec![fixture.command.run_id; 2]),
    );
    assert!(duplicate.recover_after_restart().await.is_err());
    drop(duplicate);
    let unsupported = fixture
        .orchestrator()
        .with_qualified_invocation(fixture.provider.owner_scope().unwrap());
    assert!(unsupported.recover_after_restart().await.is_err());
    assert!(unsupported.recover_stale_leases().await.is_err());
    drop(unsupported);
    assert_eq!(fixture.count("scoped-cleanup"), 0);
    assert!(fixture.cleanup.state.lock().await.target.is_none());
}

#[tokio::test]
async fn missing_qualified_cleanup_repository_never_enters_global_or_scalar_fallback() {
    let fixture = Fixture::new();
    fixture.prepare_recovery().await;
    fixture.add_leases(1);
    let sentinel = std::env::temp_dir().join(format!(
        "heph-invocation-recovery-{}.sentinel",
        fixture.command.run_id
    ));
    std::fs::write(&sentinel, b"held").unwrap();
    let orchestrator = RunOrchestrator::new(
        fixture.runs.clone(),
        fixture.volumes.clone(),
        fixture.provider.clone(),
        Arc::new(TestSpecFactory),
        32 * 1024 * 1024,
    )
    .with_runtime_manager(Arc::new(SweepSentinel(sentinel.clone())))
    .with_qualified_invocation(fixture.provider.owner_scope().unwrap());
    assert!(orchestrator.recover_after_restart().await.is_err());
    assert!(orchestrator.recover_stale_leases().await.is_err());
    drop(orchestrator);
    assert!(sentinel.exists());
    assert_eq!(lock(&fixture.volumes.stale).len(), 1);
    assert_eq!(fixture.count("recover-begin"), 0);
    std::fs::remove_file(sentinel).unwrap();
}

#[tokio::test]
async fn conflicting_legacy_and_invocation_scopes_fail_before_either_inventory() {
    let fixture = Fixture::new();
    fixture.prepare_recovery().await;
    fixture.add_leases(1);
    let scope =
        vm_trait::VmProviderOwnerScope::new(runtime_types::RunId::new().to_string(), "test".into())
            .unwrap();
    *lock(&fixture.provider.scope) = scope.clone();
    let legacy = run_domain::LegacyVmPlacementScope::new(scope).unwrap();
    let inventory = inventory(&fixture, vec![fixture.command.run_id]);
    let orchestrator = qualified(&fixture, inventory.clone()).with_legacy_vm_placement(legacy);
    assert!(orchestrator.recover_after_restart().await.is_err());
    assert!(orchestrator.recover_stale_leases().await.is_err());
    drop(orchestrator);
    assert!(lock(&inventory.cutoffs).is_empty());
    drop(inventory);
    assert_eq!(fixture.count("scoped-cleanup"), 0);
    assert_eq!(lock(&fixture.volumes.stale).len(), 1);
    assert!(fixture.cleanup.state.lock().await.target.is_none());
}
