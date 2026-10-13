//! Scalar/callback compatibility against actual private canonical repositories.

use std::sync::Arc;

use authz_postgres::PostgresMelangeAuthorizer;
use release_postgres::{ReleaseService, ReleaseServiceError, UpdateDecision, UpdateHookResult};
use run_domain::{RunCleanupHostId, RunCleanupReceipt, RunCleanupVmObservation};
use run_orchestrator::RunCleanupRepository;
use run_postgres::PgRunCleanupRepository;
use runtime_types::{RunId, VolumeId};
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;
use vm_trait::VmId;
use volume_trait::{RunVolumeMetadataRepository, VolumeLease, VolumeMetadataRepository};

use crate::{fixture, legacy, support::Fixture};

pub async fn exercise(pool: &PgPool, seeded: &Fixture) {
    let (instance, revision, volume) = legacy::consumer(pool, seeded).await;
    let run = fixture::normal_run(pool, seeded, instance, revision, seeded.release_agent).await;
    let worker = fixture::worker(pool).await;
    let (now, expiry) = fixture::times();
    let lease = acquire_and_attach(pool, &worker, volume, run).await;
    stale_and_held(pool, &worker, &lease).await;
    // Observe natural expiry; changing expires_at below acquired_at would be
    // an invalid fixture and would bypass the historical timestamp invariant.
    while OffsetDateTime::now_utc() <= lease.expires_at {
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    worker
        .begin_recovery(&lease)
        .await
        .expect("scalar recovery retains exact evidence under canonical lock order");
    assert_eq!(fixture::count(pool, run).await, 1);
    assert!(worker.heartbeat(&lease, now, expiry).await.is_err());
    assert!(worker.release_after_detach(&lease, true).await.is_err());
    complete(pool, run).await;
    worker
        .release_after_detach(&lease, false)
        .await
        .expect("already committed whole-set completion replay");
    let next = fixture::normal_run(pool, seeded, instance, revision, seeded.release_agent).await;
    let next_lease = worker
        .acquire(lease.volume_id, next, "host", now, expiry)
        .await
        .expect("new fenced acquisition");
    assert_eq!(next_lease.fencing_token, 2);
    let reused = rows(pool, next, volume).await;
    worker
        .release_after_detach(&lease, true)
        .await
        .expect("old completion replay after reuse");
    assert_eq!(
        rows(pool, next, volume).await,
        reused,
        "replay never touches newer resource generation"
    );
    println!(
        "PRIVATE_SCALAR_COMPAT=1 normal_exact_origin=1 stale_tuple_no_mutation=1 release_requires_committed_fullset=1 old_replay_no_resource_write=1 db_only=1"
    );
}

async fn acquire_and_attach(
    pool: &PgPool,
    worker: &volume_postgres::PostgresVolumeMetadataRepository,
    volume: Uuid,
    run: RunId,
) -> VolumeLease {
    let now = OffsetDateTime::now_utc();
    let expiry = now + time::Duration::milliseconds(25);
    assert!(
        worker
            .acquire(VolumeId::new(), run, "host", now, expiry)
            .await
            .is_err()
    );
    assert!(
        worker
            .acquire(VolumeId::from_uuid(volume), run, "wrong", now, expiry)
            .await
            .is_err()
    );
    assert_eq!(fixture::count(pool, run).await, 0);
    let lease = worker
        .acquire(VolumeId::from_uuid(volume), run, "host", now, expiry)
        .await
        .expect("scalar canonical acquisition");
    assert_eq!(
        lease.fencing_token, 1,
        "trigger advances once, no old generation write"
    );
    assert_eq!(
        worker
            .acquire(lease.volume_id, run, "host", now, expiry)
            .await
            .expect("scalar replay"),
        lease
    );
    let attached = worker
        .mark_attached(&lease, now, expiry)
        .await
        .expect("exact scalar attach");
    worker
        .heartbeat(&attached, now, expiry)
        .await
        .expect("exact scalar heartbeat");
    lease
}

async fn stale_and_held(
    pool: &PgPool,
    worker: &volume_postgres::PostgresVolumeMetadataRepository,
    lease: &VolumeLease,
) {
    let run = lease.run_id;
    let volume = lease.volume_id.as_uuid();
    let (now, expiry) = fixture::times();
    let before = rows(pool, run, volume).await;
    for invalid in altered(lease) {
        assert!(worker.mark_attached(&invalid, now, expiry).await.is_err());
        assert!(worker.heartbeat(&invalid, now, expiry).await.is_err());
        assert!(worker.release_after_detach(&invalid, false).await.is_err());
    }
    assert_eq!(
        rows(pool, run, volume).await,
        before,
        "stale tuple checks precede mutations"
    );
    assert!(worker.release_after_detach(lease, false).await.is_err());
    assert_eq!(
        rows(pool, run, volume).await,
        before,
        "detach caller cannot release or restore Ready"
    );
}

fn altered(lease: &VolumeLease) -> Vec<VolumeLease> {
    let mut id = lease.clone();
    id.id = runtime_types::LeaseId::new();
    let mut fence = lease.clone();
    fence.fencing_token += 1;
    let mut host = lease.clone();
    host.host_id = "wrong".into();
    let mut run = lease.clone();
    run.run_id = RunId::new();
    let mut volume = lease.clone();
    volume.volume_id = VolumeId::new();
    vec![id, fence, host, run, volume]
}

async fn complete(pool: &PgPool, run: RunId) {
    let worker_pool = fixture::worker_pool(pool).await;
    let metadata = fixture::worker(pool).await;
    let selected = metadata
        .load_run_selections(run)
        .await
        .expect("exact still admitted cleanup source");
    let host = RunCleanupHostId::new("scalar-fixture-provider".into(), "host".into())
        .expect("fixture scope");
    let cleanup = PgRunCleanupRepository::new(worker_pool.clone(), host.clone());
    let vm = VmId(format!("scalar-{run}"));
    cleanup
        .bind_vm_before_provision(
            run,
            selected.identity().instance_id(),
            selected.identity().revision_id(),
            &host,
            &vm,
        )
        .await
        .expect("fixture persisted VM scope, not physical IO");
    let target = cleanup
        .begin_cleanup(run)
        .await
        .expect("closed singleton canonical target");
    sqlx::query("UPDATE runs SET state='cleaning_up' WHERE id=$1")
        .bind(run.as_uuid())
        .execute(&worker_pool)
        .await
        .expect("explicit worker cleanup state");
    let receipt = RunCleanupReceipt::new(
        target,
        RunCleanupVmObservation::destroyed(host, vm).expect("fixture observation"),
        OffsetDateTime::now_utc(),
    )
    .expect("checked DTO only");
    cleanup
        .record_cleanup_receipt(&receipt)
        .await
        .expect("canonical worker metadata receipt");
    cleanup
        .finish_cleanup(&receipt)
        .await
        .expect("atomic whole-set completion");
}

pub async fn hook_callback(
    pool: &PgPool,
    seeded: &Fixture,
    instance: Uuid,
    update: Uuid,
    run: RunId,
    lease: &VolumeLease,
) {
    let service = ReleaseService::new(pool.clone(), Arc::new(PostgresMelangeAuthorizer));
    let before = rows(pool, run, lease.volume_id.as_uuid()).await;
    let rejected = service
        .record_update_hook_result(
            update.to_string().parse().expect("typed update identity"),
            UpdateHookResult::Rejected(23),
        )
        .await;
    assert!(
        matches!(rejected, Err(ReleaseServiceError::InvalidUpdateLifecycle)),
        "held normal hook lease must prevent callback release: {rejected:?}"
    );
    assert_eq!(rows(pool, run, lease.volume_id.as_uuid()).await, before);
    let state:(String,String,bool)=sqlx::query_as("SELECT candidate.state,consumer.state,consumer.run_gate_open FROM agent_updates candidate JOIN agent_instances consumer ON consumer.id=candidate.instance_id WHERE candidate.id=$1")
        .bind(update).fetch_one(pool).await.expect("rolled-back callback");
    assert_eq!(state, ("hook_running".into(), "updating".into(), false));
    complete(pool, run).await;
    assert_eq!(
        service
            .record_update_hook_result(
                update.to_string().parse().expect("typed update identity"),
                UpdateHookResult::Rejected(23)
            )
            .await
            .expect("callback sees committed completion"),
        UpdateDecision::AgentRejected
    );
    let current: Uuid =
        sqlx::query_scalar("SELECT active_revision_id FROM agent_instances WHERE id=$1")
            .bind(instance)
            .fetch_one(pool)
            .await
            .expect("ordinary restored legacy revision");
    let next = fixture::normal_run(pool, seeded, instance, current, seeded.release_agent).await;
    let worker = fixture::worker(pool).await;
    let (now, expiry) = fixture::times();
    worker
        .acquire(lease.volume_id, next, "host", now, expiry)
        .await
        .expect("retained volume reused");
    let reused = rows(pool, next, lease.volume_id.as_uuid()).await;
    assert_eq!(
        service
            .record_update_hook_result(
                update.to_string().parse().expect("typed update identity"),
                UpdateHookResult::Rejected(23)
            )
            .await
            .expect("terminal callback replay"),
        UpdateDecision::AgentRejected
    );
    assert_eq!(rows(pool, next, lease.volume_id.as_uuid()).await, reused);
    println!(
        "PRIVATE_SCALAR_HOOK=1 update_id_null_hook_found=1 held_callback_whole_tx_rollback=1 postcompletion_callback_replay=1 newer_fence_untouched=1 db_only=1"
    );
}

async fn rows(pool: &PgPool, run: RunId, volume: Uuid) -> Vec<serde_json::Value> {
    sqlx::query_scalar("SELECT to_jsonb(lease) FROM agent_instance_volume_leases lease WHERE run_id=$1 UNION ALL SELECT to_jsonb(volume) FROM agent_instance_state_volumes volume WHERE id=$2 ORDER BY 1")
        .bind(run.as_uuid()).bind(volume).fetch_all(pool).await.expect("exact lease/resource snapshot")
}
