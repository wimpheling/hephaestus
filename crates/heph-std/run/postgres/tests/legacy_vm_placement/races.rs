use run_domain::{LegacyVmPlacementConsumption, RunState};
use run_orchestrator::RunRepository;
use sqlx::PgPool;
use time::OffsetDateTime;
use volume_trait::VolumeMetadataRepository;

use super::fixture;

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable private placement PostgreSQL"]
async fn consume_and_permanent_close_serialize_on_same_run(pool: PgPool) {
    fixture::schema(&pool).await;
    let owner = fixture::scope();
    let (command, runs, worker) = fixture::hook(&pool, &owner).await;
    let mut tx = worker.begin().await.unwrap();
    sqlx::query("SELECT consume_legacy_vm_placement($1,$2,$3,$4,'provision')")
        .bind(command.run_id.as_uuid())
        .bind(command.command_id.as_uuid())
        .bind(
            owner
                .provider_scope()
                .namespace()
                .parse::<uuid::Uuid>()
                .unwrap(),
        )
        .bind(owner.provider_scope().host_id())
        .execute(&mut *tx)
        .await
        .unwrap();
    let closing = {
        let pool = worker.clone();
        let command = command.clone();
        tokio::spawn(async move {
            fixture::close(&pool, &command).await;
        })
    };
    let mut closing = closing;
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(30), &mut closing)
            .await
            .is_err()
    );
    tx.commit().await.unwrap();
    closing.await.unwrap();
    assert!(
        runs.consume_legacy_vm_placement(&command, &owner, LegacyVmPlacementConsumption::Provision)
            .await
            .is_err()
    );
    assert!(
        runs.transition(command.run_id, RunState::LeasingVolume, None, None)
            .await
            .is_err()
    );
    worker.close().await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable private placement PostgreSQL"]
async fn released_tuple_replay_does_not_reset_reacquired_volume_generation(pool: PgPool) {
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
    f.to_running(Some(&lease)).await;
    f.runs
        .transition(f.command.run_id, RunState::Succeeded, None, None)
        .await
        .unwrap();
    f.runs
        .transition(f.command.run_id, RunState::CleaningUp, None, None)
        .await
        .unwrap();
    // Trusted scalar release metadata only; no physical VM confirmation claim.
    f.volumes.release_after_detach(&lease, false).await.unwrap();
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
    let reused = f
        .volumes
        .acquire(
            volume,
            next.run_id,
            "lifecycle-host",
            now,
            now + time::Duration::minutes(5),
        )
        .await
        .unwrap();
    assert!(reused.fencing_token > lease.fencing_token);
    let before: serde_json::Value = sqlx::query_scalar(
        "SELECT to_jsonb(volume) FROM agent_instance_state_volumes volume WHERE id=$1",
    )
    .bind(volume.as_uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    f.runs
        .transition(f.command.run_id, RunState::CleanedUp, None, None)
        .await
        .unwrap();
    assert!(
        !f.runs
            .create_run_with_legacy_placement(&f.command, &owner)
            .await
            .unwrap()
            .created
    );
    let after: serde_json::Value = sqlx::query_scalar(
        "SELECT to_jsonb(volume) FROM agent_instance_state_volumes volume WHERE id=$1",
    )
    .bind(volume.as_uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(before, after);
    let inventory = f.runs.legacy_vm_placement_inventory(&owner).await.unwrap();
    assert!(
        inventory
            .placements
            .iter()
            .any(|plan| plan.command().run_id == f.command.run_id)
    );
    assert!(
        inventory
            .placements
            .iter()
            .any(|plan| plan.command().run_id == next.run_id)
    );
    f.worker.close().await;
}
