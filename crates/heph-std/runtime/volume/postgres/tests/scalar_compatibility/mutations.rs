use runtime_types::{RunId, VolumeId};
use sqlx::PgPool;
use volume_trait::VolumeMetadataRepository;

use super::{fixture, producers};

#[sqlx::test(migrations = false)]
#[ignore = "requires actual disposable private scalar PostgreSQL"]
async fn exact_origin_acquires_attaches_and_renews_without_canonical_snapshot(pool: PgPool) {
    let (f, e, repository, lease) = fixture::acquired(&pool).await;
    let (now, expires) = fixture::times();
    let replay = repository
        .acquire(lease.volume_id, lease.run_id, "host", now, expires)
        .await
        .unwrap();
    assert_eq!(replay.id, lease.id);
    assert_eq!(replay.fencing_token, 1);
    let attached = repository
        .mark_attached(&lease, now, expires)
        .await
        .unwrap();
    assert!(attached.attached_at.is_some());
    let (now, expires) = fixture::times();
    repository.heartbeat(&attached, now, expires).await.unwrap();
    assert_eq!(
        repository
            .active_lease_for_run(lease.run_id, "host")
            .await
            .unwrap()
            .unwrap()
            .id,
        lease.id
    );
    let row = fixture::evidence(&pool, f.volume).await;
    assert_eq!(row["volume"]["state"], "attached");
    assert_eq!(row["volume"]["lease_generation"], 1);
    assert_eq!(row["leases"][0]["selection_origin"], "legacy_origin");
    assert_eq!(
        row["leases"][0]["selected_revision_id"],
        e.revision.to_string()
    );
    assert_eq!(
        row["leases"][0]["selected_release_agent_id"],
        e.agent.to_string()
    );
    assert_eq!(row["leases"][0]["slot_key"], "state");
    let snapshot: Option<serde_json::Value> =
        sqlx::query_scalar("SELECT volume_selection_snapshot FROM runs WHERE id=$1")
            .bind(e.run)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(snapshot.is_none());
    let binding: (uuid::Uuid, uuid::Uuid, i64, Option<String>) =
        sqlx::query_as("SELECT volume_id,lease_id,lease_fencing_token,vm_id FROM runs WHERE id=$1")
            .bind(e.run)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        binding,
        (f.volume, lease.id.as_uuid(), lease.fencing_token, None)
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires actual disposable private scalar PostgreSQL"]
async fn wrong_tuple_host_and_live_recovery_leave_every_row_unchanged(pool: PgPool) {
    let (f, e, repository, lease) = fixture::acquired(&pool).await;
    let before = fixture::evidence(&pool, f.volume).await;
    let (now, expires) = fixture::times();
    let mut forged = lease.clone();
    forged.fencing_token += 1;
    assert!(
        repository
            .mark_attached(&forged, now, expires)
            .await
            .is_err()
    );
    forged = lease.clone();
    forged.run_id = RunId::new();
    assert!(
        repository
            .release_after_detach(&forged, false)
            .await
            .is_err()
    );
    assert!(
        repository
            .active_lease_for_run(RunId::from_uuid(e.run), "foreign")
            .await
            .is_err()
    );
    assert!(
        repository
            .acquire(
                VolumeId::from_uuid(f.volume),
                lease.run_id,
                "foreign",
                now,
                expires
            )
            .await
            .is_err()
    );
    assert!(repository.begin_recovery(&lease).await.is_err());
    assert_eq!(fixture::evidence(&pool, f.volume).await, before);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires actual disposable private scalar PostgreSQL"]
async fn cleanup_required_then_atomic_release_and_stable_replay(pool: PgPool) {
    let (f, e, repository, lease) = fixture::acquired(&pool).await;
    let (now, expires) = fixture::times();
    repository
        .mark_attached(&lease, now, expires)
        .await
        .unwrap();
    let before = fixture::evidence(&pool, f.volume).await;
    assert!(
        repository
            .release_after_detach(&lease, false)
            .await
            .is_err()
    );
    assert_eq!(fixture::evidence(&pool, f.volume).await, before);
    fixture::advance(&pool, e.run, "cleaning_up").await;
    repository
        .release_after_detach(&lease, false)
        .await
        .unwrap();
    let released = fixture::evidence(&pool, f.volume).await;
    assert_eq!(released["volume"]["state"], "ready");
    assert_eq!(released["volume"]["lease_generation"], 1);
    assert_eq!(released["leases"][0]["state"], "released");
    assert!(!released["leases"][0]["released_at"].is_null());
    repository
        .release_after_detach(&lease, false)
        .await
        .unwrap();
    assert_eq!(fixture::evidence(&pool, f.volume).await, released);
    fixture::advance(&pool, e.run, "cleaned_up").await;
    let receipts: i64 =
        sqlx::query_scalar("SELECT count(*) FROM run_cleanup_receipts WHERE run_id=$1")
            .bind(e.run)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        receipts, 0,
        "legacy lifecycle is never canonical absence proof"
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires actual disposable private scalar PostgreSQL"]
async fn released_replay_after_new_generation_never_mutates_reused_resource(pool: PgPool) {
    let (f, e, repository, old) = fixture::acquired(&pool).await;
    fixture::advance(&pool, e.run, "cleaning_up").await;
    repository.release_after_detach(&old, false).await.unwrap();
    fixture::advance(&pool, e.run, "cleaned_up").await;
    let next = fixture::Execution::normal(&pool, &f).await;
    producers::normal(&pool, &next, "legacy_scalar", false)
        .await
        .unwrap();
    fixture::advance(&pool, next.run, "leasing_volume").await;
    let (now, expires) = fixture::times();
    let current = repository
        .acquire(
            old.volume_id,
            RunId::from_uuid(next.run),
            "host",
            now,
            expires,
        )
        .await
        .unwrap();
    assert_eq!(current.fencing_token, old.fencing_token + 1);
    repository
        .mark_attached(&current, now, expires)
        .await
        .unwrap();
    let before = fixture::evidence(&pool, f.volume).await;
    let events: i64 =
        sqlx::query_scalar("SELECT count(*) FROM application_events WHERE aggregate_id=$1")
            .bind(f.volume)
            .fetch_one(&pool)
            .await
            .unwrap();
    repository.release_after_detach(&old, false).await.unwrap();
    assert_eq!(fixture::evidence(&pool, f.volume).await, before);
    let after: i64 =
        sqlx::query_scalar("SELECT count(*) FROM application_events WHERE aggregate_id=$1")
            .bind(f.volume)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        events, after,
        "no resource UPDATE or outbox event on duplicate replay"
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires actual disposable private scalar PostgreSQL"]
async fn frozen_cleanup_survives_release_withdrawal_without_source_grant(pool: PgPool) {
    let (f, e, repository, lease) = fixture::acquired(&pool).await;
    sqlx::query("UPDATE releases SET state='revoked',revoked_at=statement_timestamp() WHERE id=$1")
        .bind(f.release)
        .execute(&pool)
        .await
        .unwrap();
    fixture::advance(&pool, e.run, "cleaning_up").await;
    repository
        .release_after_detach(&lease, false)
        .await
        .unwrap();
    fixture::advance(&pool, e.run, "cleaned_up").await;
    assert_eq!(
        fixture::evidence(&pool, f.volume).await["volume"]["state"],
        "ready"
    );
}
