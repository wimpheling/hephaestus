//! Canonical target/receipt and separate owning completion transactions.
use super::{
    fixture::{Fixture, secrets},
    observations,
    running::ControlledGuest,
};
use heph_run::{RunAuthorityManager, RunRepository, RunState};
use run_domain::{RunCleanupReceipt, RunCleanupVmObservation};
use run_orchestrator::{RunCleanupRepository, RunSecretManager};
use run_postgres::PgRunCleanupRepository;
use vm_trait::{VmInstance, VmProvider};
use volume_trait::RunVolumeStore;

pub fn repository(f: &Fixture) -> PgRunCleanupRepository {
    PgRunCleanupRepository::new(f.birth.birth.worker.clone(), f.birth.scope.clone())
        .with_qualified_invocation_configuration(f.birth.config.clone())
}
pub async fn close_after_exit(f: &Fixture, guest: &ControlledGuest) -> RunCleanupReceipt {
    let target = repository(f)
        .begin_cleanup(f.birth.start.run_id)
        .await
        .unwrap();
    let held = f
        .volumes
        .leases_for_run(f.birth.start.run_id)
        .await
        .unwrap();
    assert_eq!(target.leases().len(), held.len());
    let mut held_by_id: std::collections::BTreeMap<_, _> =
        held.iter().map(|lease| (lease.lease().id, lease)).collect();
    assert_eq!(held_by_id.len(), held.len());
    for fence in target.leases() {
        let lease = held_by_id
            .remove(&fence.lease_id())
            .expect("target lease must belong to the original held set");
        assert_eq!(fence.lease_id(), lease.lease().id);
        assert_eq!(fence.fencing_token(), lease.lease().fencing_token);
        assert_eq!(fence.volume_id(), lease.lease().volume_id);
        assert_eq!(fence.run_id(), lease.lease().run_id);
        assert_eq!(fence.host_id(), lease.lease().host_id.as_str());
    }
    assert!(held_by_id.is_empty());
    assert_eq!(target.vm_target().vm_id(), Some(guest.id()));
    guest.destroy().await.unwrap();
    let owner = guest.owner_scope().unwrap();
    assert_eq!(owner, f.birth.config.provider_scope().clone());
    guest
        .cleanup_orphan_scoped(&owner, guest.id())
        .await
        .unwrap();
    assert_eq!(guest.owner_scope().unwrap(), owner);
    let receipt = RunCleanupReceipt::new(
        target,
        RunCleanupVmObservation::destroyed(f.birth.scope.clone(), guest.id().clone()).unwrap(),
        crate::qualified_invocation::fixture::pg_time(),
    )
    .unwrap();
    repository(f)
        .record_cleanup_receipt(&receipt)
        .await
        .unwrap();
    f.manager()
        .revoke_after_guest(f.birth.start.run_id)
        .await
        .unwrap();
    secrets(f)
        .destroy_after_guest(f.birth.start.run_id)
        .await
        .unwrap();
    assert_eq!(
        guest.calls(),
        ["start", "wait", "destroy", "scoped_confirmation"]
    );
    assert_eq!(f.run().await.state, RunState::Running);
    receipt
}
pub async fn cleaning(f: &Fixture) {
    let old = f.run().await;
    assert_eq!(old.state, RunState::Succeeded);
    let next = f
        .birth
        .repository()
        .transition(old.id, RunState::CleaningUp, None, None)
        .await
        .unwrap();
    assert_eq!(next.state_version, old.state_version + 1);
    assert_eq!(next.exit, old.exit);
    assert_eq!(next.outcome, old.outcome);
    assert_eq!(next.failure, old.failure);
    assert_eq!(
        f.run().await,
        next,
        "separate owning transaction committed CleaningUp"
    );
}
pub async fn finish(f: &Fixture, receipt: &RunCleanupReceipt) {
    let before = f.run().await;
    assert_eq!(before.state, RunState::CleaningUp);
    let completed = repository(f).finish_cleanup(receipt).await.unwrap();
    assert_eq!(completed.state, RunState::CleanedUp);
    assert_eq!(completed.state_version, before.state_version + 1);
    assert_eq!(completed.exit, before.exit);
    assert_eq!(completed.outcome, before.outcome);
    assert_eq!(completed.failure, before.failure);
    assert_eq!(f.run().await, completed);
    observations::completed(f, receipt).await;
    let rows = observations::rows(f).await;
    assert_eq!(
        repository(f).finish_cleanup(receipt).await.unwrap(),
        completed
    );
    assert_eq!(observations::rows(f).await, rows);
    f.birth.physical.assert_unchanged();
}
