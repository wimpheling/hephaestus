//! GLOBAL original rows, including expiry/recovery and released-fence reuse.
use super::{
    base, fixture,
    public_fixture::{Role, role_pool},
};
use run_domain::{LegacyVmPlacementScope, RunState};
use run_orchestrator::RunRepository;
use run_postgres::PgRunRepository;
use sqlx::PgPool;
use time::OffsetDateTime;
use volume_postgres::{PostgresVolumeMetadataRepository, ScalarMetadataSchema};
use volume_trait::{ScalarLeaseHistory, VolumeLease, VolumeMetadataRepository};
pub async fn acquire(f: &base::Fixture, owner: &LegacyVmPlacementScope) -> VolumeLease {
    acquire_with_ttl(f, owner, time::Duration::minutes(5)).await
}
async fn acquire_with_ttl(
    f: &base::Fixture,
    owner: &LegacyVmPlacementScope,
    ttl: time::Duration,
) -> VolumeLease {
    f.runs
        .create_run_with_legacy_placement(&f.command, owner)
        .await
        .unwrap();
    f.runs
        .transition(f.command.run_id, RunState::LeasingVolume, None, None)
        .await
        .unwrap();
    let now = OffsetDateTime::now_utc();
    f.volumes
        .acquire(
            f.volume.unwrap(),
            f.command.run_id,
            owner.provider_scope().host_id(),
            now,
            now + ttl,
        )
        .await
        .unwrap()
}
#[sqlx::test(migrations = false)]
#[ignore = "requires disposable private strict Legacy PostgreSQL"]
async fn worker_open_check_rejects_role_scope_and_permanent_closure(pool: PgPool) {
    fixture::schema(&pool).await;
    let owner = fixture::scope();
    let f = fixture::fresh(&pool, false, &owner).await;
    f.runs
        .create_run_with_legacy_placement(&f.command, &owner)
        .await
        .unwrap();
    let plan = f
        .runs
        .assert_legacy_vm_placement_open(f.command.run_id, &owner)
        .await
        .unwrap();
    assert_eq!(plan.command(), &f.command);
    assert!(
        f.runs
            .assert_legacy_vm_placement_open(f.command.run_id, &fixture::scope())
            .await
            .is_err()
    );
    let actor = role_pool(&pool, Role::Actor).await;
    let actor_runs = PgRunRepository::new(actor.clone())
        .with_execution_profile(run_domain::FreshRunExecutionProfile::LegacyScalar)
        .with_legacy_vm_placement(owner.clone());
    assert!(
        actor_runs
            .assert_legacy_vm_placement_open(f.command.run_id, &owner)
            .await
            .is_err()
    );
    f.runs
        .close_legacy_vm_acquisition(f.command.run_id, &owner)
        .await
        .unwrap();
    assert!(
        f.runs
            .assert_legacy_vm_placement_open(f.command.run_id, &owner)
            .await
            .is_err()
    );
    let inventory = f.runs.legacy_vm_placement_inventory(&owner).await.unwrap();
    assert_eq!(inventory.closed_runs, vec![f.command.run_id]);
    assert_eq!(inventory.placements, vec![plan]);
    actor.close().await;
    f.worker.close().await;
}
#[sqlx::test(migrations = false)]
#[ignore = "requires disposable private strict Legacy PostgreSQL"]
async fn global_zero_history_is_positive_metadata_not_vm_absence(pool: PgPool) {
    fixture::schema(&pool).await;
    let owner = fixture::scope();
    let f = fixture::fresh(&pool, false, &owner).await;
    f.runs
        .create_run_with_legacy_placement(&f.command, &owner)
        .await
        .unwrap();
    assert_eq!(
        f.volumes
            .scalar_lease_history(f.command.run_id)
            .await
            .unwrap(),
        ScalarLeaseHistory::NoHistory
    );
    assert!(
        f.runs
            .legacy_vm_placement(fixture::command().run_id, &owner)
            .await
            .is_err()
    );
    assert!(
        f.runs
            .legacy_vm_cleanup_completed(f.command.run_id, &owner)
            .await
            .is_err()
    );
    assert!(
        f.runs
            .legacy_vm_placement_inventory(&owner)
            .await
            .unwrap()
            .closed_runs
            .is_empty()
    );
    let actor = role_pool(&pool, Role::Actor).await;
    let metadata = PostgresVolumeMetadataRepository::new(actor.clone())
        .with_scalar_schema(ScalarMetadataSchema::Profiled);
    assert!(
        metadata
            .scalar_lease_history(f.command.run_id)
            .await
            .is_err()
    );
    assert!(f.runs.get(f.command.run_id).await.unwrap().vm_id.is_none());
    actor.close().await;
    f.worker.close().await;
}
#[sqlx::test(migrations = false)]
#[ignore = "requires disposable private strict Legacy PostgreSQL"]
async fn expired_active_row_remains_held_until_actual_recovery_marker(pool: PgPool) {
    fixture::schema(&pool).await;
    let owner = fixture::scope();
    let f = fixture::fresh(&pool, true, &owner).await;
    let lease = acquire_with_ttl(&f, &owner, time::Duration::seconds(2)).await;
    // Wait for actual expiry without mutating frozen timestamps or guards.
    sqlx::query("SELECT pg_sleep(GREATEST(0,EXTRACT(epoch FROM expires_at-clock_timestamp()))::double precision+0.01) FROM agent_instance_volume_leases WHERE id=$1")
        .bind(lease.id.as_uuid()).execute(&pool).await.unwrap();
    let ScalarLeaseHistory::Held(expired) = f
        .volumes
        .scalar_lease_history(f.command.run_id)
        .await
        .unwrap()
    else {
        panic!("expiry alone must not infer recovery")
    };
    assert!(expired.expires_at < OffsetDateTime::now_utc());
    f.volumes.begin_recovery(&expired).await.unwrap();
    let ScalarLeaseHistory::HeldRecovering(marked) = f
        .volumes
        .scalar_lease_history(f.command.run_id)
        .await
        .unwrap()
    else {
        panic!("persisted recovering row required")
    };
    assert_eq!(marked, expired);
    assert!(
        f.runs
            .assert_legacy_vm_placement_open(f.command.run_id, &owner)
            .await
            .is_err()
    );
    let inventory = f.runs.legacy_vm_placement_inventory(&owner).await.unwrap();
    assert_eq!(inventory.closed_runs, vec![f.command.run_id]);
    f.worker.close().await;
}
#[sqlx::test(migrations = false)]
#[ignore = "requires disposable private strict Legacy PostgreSQL"]
async fn original_released_tuple_and_terminal_lookup_never_reset_new_owner_fence(pool: PgPool) {
    fixture::schema(&pool).await;
    let owner = fixture::scope();
    let f = fixture::fresh(&pool, true, &owner).await;
    let old = acquire(&f, &owner).await;
    f.to_running(Some(&old)).await;
    f.runs
        .transition(f.command.run_id, RunState::Succeeded, None, None)
        .await
        .unwrap();
    f.runs
        .transition(f.command.run_id, RunState::CleaningUp, None, None)
        .await
        .unwrap();
    // Trusted scalar metadata callback only, not physical VM proof.
    f.volumes.release_after_detach(&old, false).await.unwrap();
    let next = super::support::retry_command(&f.command);
    let (_, repository) = super::support::seed_run_request_scope(&pool, &next).await;
    super::support::seed_retry_request(&pool, &next, repository, f.command.run_id).await;
    f.runs
        .create_run_with_legacy_placement(&next, &owner)
        .await
        .unwrap();
    f.runs
        .transition(next.run_id, RunState::LeasingVolume, None, None)
        .await
        .unwrap();
    let now = OffsetDateTime::now_utc();
    let fresh = f
        .volumes
        .acquire(
            old.volume_id,
            next.run_id,
            owner.provider_scope().host_id(),
            now,
            now + time::Duration::minutes(5),
        )
        .await
        .unwrap();
    assert!(fresh.fencing_token > old.fencing_token);
    let before = base::evidence(&pool, next.run_id).await;
    let ScalarLeaseHistory::Released(original) = f
        .volumes
        .scalar_lease_history(f.command.run_id)
        .await
        .unwrap()
    else {
        panic!("actual original released history required")
    };
    assert_eq!(original, old);
    f.runs
        .transition(f.command.run_id, RunState::CleanedUp, None, None)
        .await
        .unwrap();
    let done = f
        .runs
        .legacy_vm_cleanup_completed(f.command.run_id, &owner)
        .await
        .unwrap();
    assert_eq!(done.state, RunState::CleanedUp);
    assert_eq!(
        f.volumes
            .scalar_lease_history(f.command.run_id)
            .await
            .unwrap(),
        ScalarLeaseHistory::Released(old)
    );
    assert_eq!(base::evidence(&pool, next.run_id).await, before);
    f.worker.close().await;
}
