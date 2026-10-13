//! Real metadata/cleanup adapter interoperability; observations remain fixtures.

use run_domain::{
    RunCleanupHostId, RunCleanupLeaseFence, RunCleanupReceipt, RunCleanupTarget,
    RunCleanupVmObservation,
};
use run_orchestrator::RunCleanupRepository;
use run_postgres::PgRunCleanupRepository;
use runtime_types::{LeaseId, VolumeId};
use sqlx::PgPool;
use time::OffsetDateTime;
use vm_trait::VmId;
use volume_trait::RunVolumeMetadataRepository;

use super::named_fixture;
use crate::{fixture, support::Fixture};

pub async fn exercise(pool: &PgPool, seeded: &Fixture) {
    let (instance, revision, agent) = named_fixture::consumer(pool, seeded, 2).await;
    let run = named_fixture::try_run(pool, seeded, instance, revision, agent)
        .await
        .expect("two-slot supported-predicate fixture");
    let metadata = fixture::worker(pool).await;
    let selected = metadata
        .load_run_selections(run)
        .await
        .expect("exact two-slot graph");
    let worker_pool = fixture::worker_pool(pool).await;
    let host = RunCleanupHostId::new("fixture-provider".into(), "host".into())
        .expect("persistent test-provider scope");
    let cleanup = PgRunCleanupRepository::new(worker_pool.clone(), host.clone());
    let vm = VmId("two-slot-fixture".into());
    cleanup
        .bind_vm_before_provision(
            run,
            selected.identity().instance_id(),
            selected.identity().revision_id(),
            &host,
            &vm,
        )
        .await
        .expect("exact VM scope persisted before simulated IO");
    let (now, expiry) = fixture::times();
    let leases = metadata
        .acquire_run(&selected, "host", now, expiry)
        .await
        .expect("two immutable canonical leases");
    assert_eq!(leases.len(), 2);
    metadata
        .mark_run_attached(&selected, "host", now, expiry)
        .await
        .expect("full-set attachment");
    let target = cleanup
        .begin_cleanup(run)
        .await
        .expect("one canonical closed complete target");
    assert_eq!(target.leases().len(), 2);
    assert!(
        metadata
            .acquire_run(&selected, "host", now, expiry)
            .await
            .is_err()
    );
    deny_changed_fences(&cleanup, &target, &host, &vm, now).await;
    let receipt = RunCleanupReceipt::new(
        target.clone(),
        RunCleanupVmObservation::destroyed(host, vm).expect("exact fixture VM"),
        OffsetDateTime::now_utc(),
    )
    .expect("syntactic worker fixture observation, not physical proof");
    cleanup
        .record_cleanup_receipt(&receipt)
        .await
        .expect("persist exact worker fixture receipt");
    assert_eq!(
        cleanup
            .recorded_cleanup_receipt(run)
            .await
            .expect("persisted canonical lookup"),
        Some(receipt.clone())
    );
    verify_atomic_completion(pool, &worker_pool, &cleanup, &receipt, &leases).await;
    cleanup
        .finish_cleanup(&receipt)
        .await
        .expect("idempotent old completion replay");
    let rows:i64=sqlx::query_scalar("SELECT count(*) FROM agent_instance_state_volumes WHERE id=ANY($1) AND state='ready' AND provisioning_state='ready'")
        .bind(leases.iter().map(|item|item.lease().volume_id.as_uuid()).collect::<Vec<_>>()).fetch_one(pool).await.expect("all resources ready after canonical completion");
    assert_eq!(rows, 2);
    completion_replay_after_reuse(pool, seeded, &cleanup, &receipt).await;
    println!(
        "PRIVATE_108_CROSS_ADAPTER=1 exact_two_slot_target=1 missing_extra_fence_denied=1 partial_release_rollback=1 stale_fence_all_held=1 atomic_complete_release=1 completion_replay_after_new_consumer=1 worker_fixture_not_physical_destruction=1"
    );
}

async fn completion_replay_after_reuse(
    pool: &PgPool,
    seeded: &Fixture,
    cleanup: &PgRunCleanupRepository,
    receipt: &RunCleanupReceipt,
) {
    let volumes = receipt
        .target()
        .leases()
        .iter()
        .map(|lease| lease.volume_id().as_uuid())
        .collect::<Vec<_>>();
    let (instance, revision, agent) = named_fixture::reuse_consumer(pool, seeded, &volumes).await;
    let next_run = named_fixture::try_run(pool, seeded, instance, revision, agent)
        .await
        .expect("new consumer reuses retained resources");
    let metadata = fixture::worker(pool).await;
    let selected = metadata
        .load_run_selections(next_run)
        .await
        .expect("new exact selection");
    let (now, expiry) = fixture::times();
    let next = metadata
        .acquire_run(&selected, "host", now, expiry)
        .await
        .expect("fresh fenced leases");
    assert_eq!(next.len(), volumes.len());
    let before = reuse_rows(pool, next_run, &volumes).await;
    cleanup
        .finish_cleanup(receipt)
        .await
        .expect("historical completion replays without resource mutation");
    assert_eq!(reuse_rows(pool, next_run, &volumes).await, before);
    assert_eq!(
        metadata
            .leases_for_run(next_run, "host")
            .await
            .expect("complete new held set"),
        next
    );
}

async fn reuse_rows(
    pool: &PgPool,
    run: runtime_types::RunId,
    volumes: &[uuid::Uuid],
) -> Vec<serde_json::Value> {
    sqlx::query_scalar("SELECT to_jsonb(lease) FROM agent_instance_volume_leases lease WHERE run_id=$1 UNION ALL SELECT to_jsonb(volume) FROM agent_instance_state_volumes volume WHERE id=ANY($2) ORDER BY 1")
        .bind(run.as_uuid()).bind(volumes).fetch_all(pool).await.expect("exact new leases and provider generation snapshot")
}

fn changed(target: &RunCleanupTarget, leases: Vec<RunCleanupLeaseFence>) -> RunCleanupTarget {
    RunCleanupTarget::new(
        target.run_id(),
        target.instance_id(),
        target.revision_id(),
        target.vm_target().clone(),
        target.generation(),
        leases,
    )
    .expect("changed complete-set shape")
}

async fn deny_changed_fences(
    cleanup: &PgRunCleanupRepository,
    target: &RunCleanupTarget,
    host: &RunCleanupHostId,
    vm: &VmId,
    now: OffsetDateTime,
) {
    let missing = changed(target, target.leases()[..1].to_vec());
    let mut extra = target.leases().to_vec();
    extra.push(
        RunCleanupLeaseFence::new(
            target.run_id(),
            LeaseId::new(),
            VolumeId::new(),
            "host".into(),
            1,
        )
        .expect("syntactically valid extra fence"),
    );
    let extra = changed(target, extra);
    for invalid in [missing, extra] {
        let receipt = RunCleanupReceipt::new(
            invalid,
            RunCleanupVmObservation::destroyed(host.clone(), vm.clone()).expect("exact fixture VM"),
            now,
        )
        .expect("fixture observation shape");
        assert!(
            cleanup.record_cleanup_receipt(&receipt).await.is_err(),
            "constructor cannot grant authority over missing or extra canonical fences"
        );
    }
}

async fn verify_atomic_completion(
    pool: &PgPool,
    worker_pool: &PgPool,
    cleanup: &PgRunCleanupRepository,
    receipt: &RunCleanupReceipt,
    leases: &[volume_trait::RunVolumeLease],
) {
    let run = receipt.target().run_id();
    let partial = sqlx::query(
        "UPDATE agent_instance_volume_leases SET state='released',released_at=now() WHERE id=$1",
    )
    .bind(leases[0].lease().id.as_uuid())
    .execute(worker_pool)
    .await
    .expect_err("one-of-two release cannot commit");
    assert_eq!(
        partial
            .as_database_error()
            .and_then(sqlx::error::DatabaseError::code)
            .as_deref(),
        Some("23000")
    );
    assert_eq!(
        fixture::count(pool, run).await,
        2,
        "partial release rolls back"
    );
    sqlx::query("UPDATE runs SET state='cleaning_up' WHERE id=$1")
        .bind(run.as_uuid())
        .execute(worker_pool)
        .await
        .expect("worker enters cleanup after closing acquisition");
    let failed = leases[0].lease();
    sqlx::query(
        "UPDATE agent_instance_state_volumes SET lease_generation=lease_generation+1 WHERE id=$1",
    )
    .bind(failed.volume_id.as_uuid())
    .execute(pool)
    .await
    .expect("simulate stale resource fence");
    assert!(
        cleanup.finish_cleanup(receipt).await.is_err(),
        "stale member prevents complete-set mutation"
    );
    assert_eq!(
        fixture::count(pool, run).await,
        2,
        "failed CAS retains every lease"
    );
    sqlx::query("UPDATE agent_instance_state_volumes SET lease_generation=$2 WHERE id=$1")
        .bind(failed.volume_id.as_uuid())
        .bind(failed.fencing_token)
        .execute(pool)
        .await
        .expect("restore fixture corruption, no production fence reuse claim");
    cleanup
        .finish_cleanup(receipt)
        .await
        .expect("atomic all-member release and CleanedUp");
    assert_eq!(fixture::count(pool, run).await, 0);
}
