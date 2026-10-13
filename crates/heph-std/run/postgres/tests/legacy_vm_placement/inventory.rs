use run_domain::{LegacyVmPlacementConsumption, LegacyVmPlacementHoldReason, RunState};
use run_orchestrator::RunRepository;
use sqlx::PgPool;
use time::OffsetDateTime;
use volume_trait::VolumeMetadataRepository;

use super::fixture;

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable private placement PostgreSQL"]
async fn scoped_inventory_retains_vm_null_unconsumed_foreign_and_unknown_evidence(pool: PgPool) {
    fixture::schema(&pool).await;
    let owner = fixture::scope();
    let local = fixture::fresh(&pool, false, &owner).await;
    local
        .runs
        .create_run_with_legacy_placement(&local.command, &owner)
        .await
        .unwrap();
    let (eligible, _, hook_worker) = fixture::hook(&pool, &owner).await;
    let foreign_owner = fixture::scope();
    let foreign = fixture::fresh(&pool, false, &foreign_owner).await;
    foreign
        .runs
        .create_run_with_legacy_placement(&foreign.command, &foreign_owner)
        .await
        .unwrap();
    let unknown = super::base::Fixture::new(&pool, false).await;
    unknown.runs.create_run(&unknown.command).await.unwrap();
    let historical = super::base::Fixture::new(&pool, false).await;
    run_postgres::PgRunRepository::new(historical.worker.clone())
        .create_run(&historical.command)
        .await
        .unwrap();
    let inventory = local
        .runs
        .legacy_vm_placement_inventory(&owner)
        .await
        .unwrap();
    assert!(
        inventory
            .placements
            .iter()
            .any(|p| p.command().run_id == local.command.run_id
                && p.consumption() == Some(LegacyVmPlacementConsumption::Provision))
    );
    assert!(
        inventory
            .placements
            .iter()
            .any(|p| p.command().run_id == eligible.run_id && p.consumption().is_none())
    );
    assert!(
        inventory
            .held
            .iter()
            .any(|h| h.run_id == foreign.command.run_id
                && h.reason == LegacyVmPlacementHoldReason::OwnerMismatch)
    );
    assert!(
        inventory
            .held
            .iter()
            .any(|h| h.run_id == unknown.command.run_id
                && h.reason == LegacyVmPlacementHoldReason::MissingBirth)
    );
    assert!(
        inventory
            .held
            .iter()
            .any(|h| h.run_id == historical.command.run_id
                && h.reason == LegacyVmPlacementHoldReason::HistoricalUnproven)
    );
    assert!(
        local
            .runs
            .get(local.command.run_id)
            .await
            .unwrap()
            .vm_id
            .is_none()
    );
    hook_worker.close().await;
    foreign.worker.close().await;
    unknown.worker.close().await;
    historical.worker.close().await;
    local.worker.close().await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable private placement PostgreSQL"]
async fn close_is_exact_idempotent_and_preserves_every_held_fence(pool: PgPool) {
    fixture::schema(&pool).await;
    let owner = fixture::scope();
    let f = fixture::fresh(&pool, true, &owner).await;
    f.runs
        .create_run_with_legacy_placement(&f.command, &owner)
        .await
        .unwrap();
    f.runs
        .transition(f.command.run_id, RunState::LeasingVolume, None, None)
        .await
        .unwrap();
    let now = OffsetDateTime::now_utc();
    let held = f
        .volumes
        .acquire(
            f.volume.unwrap(),
            f.command.run_id,
            "lifecycle-host",
            now,
            now + time::Duration::minutes(5),
        )
        .await
        .unwrap();
    assert!(
        f.runs
            .close_legacy_vm_acquisition(f.command.run_id, &fixture::scope())
            .await
            .is_err()
    );
    let first = f
        .runs
        .close_legacy_vm_acquisition(f.command.run_id, &owner)
        .await
        .unwrap();
    let evidence = super::base::evidence(&pool, f.command.run_id).await;
    let second = f
        .runs
        .close_legacy_vm_acquisition(f.command.run_id, &owner)
        .await
        .unwrap();
    assert_eq!(first, second);
    assert_eq!(
        evidence,
        super::base::evidence(&pool, f.command.run_id).await
    );
    let original = f
        .volumes
        .active_lease_for_run(f.command.run_id, "lifecycle-host")
        .await
        .unwrap();
    assert_eq!(original, Some(held));
    let inventory = f.runs.legacy_vm_placement_inventory(&owner).await.unwrap();
    assert!(
        inventory
            .placements
            .iter()
            .any(|p| p.command().run_id == f.command.run_id)
    );
    f.worker.close().await;
}
