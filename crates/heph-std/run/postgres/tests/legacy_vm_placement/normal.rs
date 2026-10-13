use run_domain::{LegacyVmPlacementConsumption, RunState};
use run_orchestrator::RunRepository;
use sqlx::PgPool;
use time::OffsetDateTime;
use volume_trait::VolumeMetadataRepository;

use super::fixture;

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable private placement PostgreSQL"]
async fn normal_birth_commits_consumed_scope_before_any_vm_or_volume_binding(pool: PgPool) {
    fixture::schema(&pool).await;
    let owner = fixture::scope();
    let f = fixture::fresh(&pool, true, &owner).await;
    let created = f
        .runs
        .create_run_with_legacy_placement(&f.command, &owner)
        .await
        .unwrap();
    assert!(created.created);
    assert!(created.run.vm_id.is_none() && created.run.volume_id.is_none());
    let plan = f
        .runs
        .legacy_vm_placement(f.command.run_id, &owner)
        .await
        .unwrap();
    assert_eq!(plan.command(), &f.command);
    assert_eq!(plan.scope(), &owner);
    assert_eq!(plan.vm_id().0, f.command.run_id.to_string());
    assert_eq!(
        plan.consumption(),
        Some(LegacyVmPlacementConsumption::Provision)
    );
    let replay = f
        .runs
        .create_run_with_legacy_placement(&f.command, &owner)
        .await
        .unwrap();
    assert!(!replay.created);
    assert_eq!(
        plan,
        f.runs
            .legacy_vm_placement(f.command.run_id, &owner)
            .await
            .unwrap()
    );
    let mut different = f.command.clone();
    different.requires_state = false;
    assert!(
        f.runs
            .create_run_with_legacy_placement(&different, &owner)
            .await
            .is_err()
    );
    assert_eq!(fixture::rows(&pool, f.command.run_id).await, 1);
    f.worker.close().await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable private placement PostgreSQL"]
async fn bad_new_pins_and_wrong_configured_scope_have_no_run_or_inbox_effects(pool: PgPool) {
    fixture::schema(&pool).await;
    let owner = fixture::scope();
    let f = fixture::fresh(&pool, false, &owner).await;
    let mut substituted = f.command.clone();
    substituted.requires_state = true;
    assert!(
        f.runs
            .create_run_with_legacy_placement(&substituted, &owner)
            .await
            .is_err()
    );
    assert!(
        f.runs
            .create_run_with_legacy_placement(&f.command, &fixture::scope())
            .await
            .is_err()
    );
    assert_eq!(fixture::rows(&pool, f.command.run_id).await, 0);
    let inbox: i64 = sqlx::query_scalar("SELECT count(*) FROM command_inbox WHERE command_id=$1")
        .bind(f.command.command_id.as_uuid())
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(inbox, 0);
    f.worker.close().await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable private placement PostgreSQL"]
async fn recorded_owner_required_for_bind_and_lease_host_matches_before_fence_changes(
    pool: PgPool,
) {
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
    let volume = f.volume.unwrap();
    let now = OffsetDateTime::now_utc();
    let generation: i64 =
        sqlx::query_scalar("SELECT lease_generation FROM agent_instance_state_volumes WHERE id=$1")
            .bind(volume.as_uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(
        f.volumes
            .acquire(
                volume,
                f.command.run_id,
                "foreign-host",
                now,
                now + time::Duration::minutes(5)
            )
            .await
            .is_err()
    );
    let after: i64 =
        sqlx::query_scalar("SELECT lease_generation FROM agent_instance_state_volumes WHERE id=$1")
            .bind(volume.as_uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(after, generation);
    let lease = f
        .volumes
        .acquire(
            volume,
            f.command.run_id,
            "lifecycle-host",
            now,
            now + time::Duration::minutes(5),
        )
        .await
        .unwrap();
    assert!(
        f.runs
            .clone()
            .with_legacy_vm_placement(fixture::scope())
            .bind_resources(
                f.command.run_id,
                Some(volume),
                Some(lease.id),
                Some(lease.fencing_token),
                &f.command.run_id.to_string()
            )
            .await
            .is_err()
    );
    assert!(f.runs.get(f.command.run_id).await.unwrap().vm_id.is_none());
    f.bind(Some(&lease)).await;
    f.worker.close().await;
}
