use run_domain::{LegacyVmPlacementConsumption, RunState};
use run_orchestrator::RunRepository;
use sqlx::PgPool;

use super::fixture;

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable private placement PostgreSQL"]
async fn update_birth_is_unconsumed_then_exact_worker_start_consumes_without_vm_io(pool: PgPool) {
    fixture::schema(&pool).await;
    let owner = fixture::scope();
    let (command, runs, worker) = fixture::hook(&pool, &owner).await;
    let eligible = runs
        .legacy_vm_placement(command.run_id, &owner)
        .await
        .unwrap();
    assert!(eligible.consumption().is_none());
    assert!(
        runs.bind_resources(
            command.run_id,
            None,
            None,
            None,
            &command.run_id.to_string()
        )
        .await
        .is_err()
    );
    let created = runs
        .create_run_with_legacy_placement(&command, &owner)
        .await
        .unwrap();
    assert!(created.created && created.run.vm_id.is_none());
    let consumed = runs
        .legacy_vm_placement(command.run_id, &owner)
        .await
        .unwrap();
    assert_eq!(
        consumed.consumption(),
        Some(LegacyVmPlacementConsumption::Provision)
    );
    assert_eq!(eligible.scope(), consumed.scope());
    assert!(
        !runs
            .create_run_with_legacy_placement(&command, &owner)
            .await
            .unwrap()
            .created
    );
    worker.close().await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable private placement PostgreSQL"]
async fn closed_unconsumed_hook_can_consume_only_cleanup_after_source_role_withdrawal(
    pool: PgPool,
) {
    fixture::schema(&pool).await;
    let owner = fixture::scope();
    let (command, runs, worker) = fixture::hook(&pool, &owner).await;
    fixture::close(&worker, &command).await;
    let global_history: i64 = sqlx::query_scalar("SELECT count(*) FROM agent_instance_volume_leases WHERE run_id=$1 OR update_id IN (SELECT id FROM agent_updates WHERE hook_run_id=$1)")
        .bind(command.run_id.as_uuid()).fetch_one(&pool).await.unwrap();
    assert_eq!(global_history, 0);
    sqlx::query("DELETE FROM project_maintainers WHERE user_id=(SELECT actor_id FROM run_legacy_vm_placements WHERE run_id=$1)")
        .bind(command.run_id.as_uuid()).execute(&pool).await.unwrap();
    assert!(
        runs.consume_legacy_vm_placement(&command, &owner, LegacyVmPlacementConsumption::Provision)
            .await
            .is_err()
    );
    let plan = runs
        .consume_legacy_vm_placement(&command, &owner, LegacyVmPlacementConsumption::CleanupOnly)
        .await
        .unwrap();
    assert_eq!(
        plan.consumption(),
        Some(LegacyVmPlacementConsumption::CleanupOnly)
    );
    assert!(
        runs.consume_legacy_vm_placement(&command, &owner, LegacyVmPlacementConsumption::Provision)
            .await
            .is_err()
    );
    assert!(
        runs.create_run_with_legacy_placement(&command, &owner)
            .await
            .is_err()
    );
    assert!(
        runs.transition(command.run_id, RunState::LeasingVolume, None, None)
            .await
            .is_err()
    );
    assert!(runs.get(command.run_id).await.unwrap().vm_id.is_none());
    let receipts: i64 =
        sqlx::query_scalar("SELECT count(*) FROM run_cleanup_receipts WHERE run_id=$1")
            .bind(command.run_id.as_uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(receipts, 0, "placement is not canonical physical evidence");
    worker.close().await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable private placement PostgreSQL"]
async fn unexpected_io_evidence_blocks_cleanup_only_without_mutating_eligibility(pool: PgPool) {
    fixture::schema(&pool).await;
    let owner = fixture::scope();
    let (command, runs, worker) = fixture::hook(&pool, &owner).await;
    fixture::close(&worker, &command).await;
    sqlx::query("INSERT INTO run_events(id,run_id,sequence,event_type,payload,occurred_at) VALUES(gen_random_uuid(),$1,900,'vm.started','{}',now())")
        .bind(command.run_id.as_uuid()).execute(&pool).await.unwrap();
    assert!(
        runs.consume_legacy_vm_placement(
            &command,
            &owner,
            LegacyVmPlacementConsumption::CleanupOnly
        )
        .await
        .is_err()
    );
    assert!(
        runs.legacy_vm_placement(command.run_id, &owner)
            .await
            .unwrap()
            .consumption()
            .is_none()
    );
    worker.close().await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable private placement PostgreSQL"]
async fn changing_mutable_update_actor_does_not_change_recorded_birth_provenance(pool: PgPool) {
    fixture::schema(&pool).await;
    let owner = fixture::scope();
    let (command, runs, worker) = fixture::hook(&pool, &owner).await;
    let plan = runs
        .legacy_vm_placement(command.run_id, &owner)
        .await
        .unwrap();
    sqlx::query("WITH fresh AS (INSERT INTO users(id,display_name) VALUES(gen_random_uuid(),'Unrelated fixture actor') RETURNING id) UPDATE agent_updates SET actor_id=(SELECT id FROM fresh) WHERE hook_run_id=$1")
        .bind(command.run_id.as_uuid()).execute(&pool).await.unwrap();
    assert_eq!(
        plan,
        runs.legacy_vm_placement(command.run_id, &owner)
            .await
            .unwrap()
    );
    worker.close().await;
}
