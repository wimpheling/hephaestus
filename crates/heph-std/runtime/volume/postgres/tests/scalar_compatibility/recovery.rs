use sqlx::PgPool;
use volume_trait::VolumeMetadataRepository;

use super::fixture;

#[sqlx::test(migrations = false)]
#[ignore = "requires actual disposable private scalar PostgreSQL"]
async fn actual_expiry_closes_recovery_but_holds_until_trusted_detach(pool: PgPool) {
    let (f, e, repository, lease) =
        fixture::acquired_with_ttl(&pool, time::Duration::seconds(2)).await;
    fixture::expire(&pool, &lease).await;
    let stale = repository
        .stale_leases(time::OffsetDateTime::now_utc())
        .await
        .unwrap();
    assert_eq!(stale.len(), 1);
    assert_eq!(stale[0].id, lease.id);
    repository.begin_recovery(&lease).await.unwrap();
    let row = fixture::evidence(&pool, f.volume).await;
    assert_eq!(row["volume"]["state"], "recovering");
    assert_eq!(row["leases"][0]["state"], "recovery_required");
    assert!(row["leases"][0]["released_at"].is_null());
    let closed: bool =
        sqlx::query_scalar("SELECT volume_acquisition_closed_at IS NOT NULL FROM runs WHERE id=$1")
            .bind(e.run)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(closed);
    let (now, expires) = fixture::times();
    assert!(repository.heartbeat(&lease, now, expires).await.is_err());
    assert!(
        repository
            .acquire(lease.volume_id, lease.run_id, "host", now, expires)
            .await
            .is_err()
    );
    assert_eq!(fixture::evidence(&pool, f.volume).await, row);
    // Trusted scalar provider fixture: SQL lifecycle semantics only. No actual
    // VM was destroyed, and no canonical absence receipt is asserted.
    repository.release_after_detach(&lease, true).await.unwrap();
    assert_eq!(
        fixture::evidence(&pool, f.volume).await["volume"]["state"],
        "ready"
    );
    let state: String = sqlx::query_scalar("SELECT state FROM runs WHERE id=$1")
        .bind(e.run)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        state, "leasing_volume",
        "recovery release precedes later core CleaningUp"
    );
    fixture::advance(&pool, e.run, "cleaning_up").await;
    fixture::advance(&pool, e.run, "cleaned_up").await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires actual disposable private scalar PostgreSQL"]
async fn expired_heartbeat_and_raw_renewal_cannot_defeat_recovery(pool: PgPool) {
    let (f, _, repository, lease) =
        fixture::acquired_with_ttl(&pool, time::Duration::seconds(2)).await;
    fixture::expire(&pool, &lease).await;
    let before = fixture::evidence(&pool, f.volume).await;
    let (now, expires) = fixture::times();
    assert!(repository.heartbeat(&lease, now, expires).await.is_err());
    assert!(
        repository
            .mark_attached(&lease, now, expires)
            .await
            .is_err()
    );
    let mut tx = fixture::worker(&pool).await;
    assert!(sqlx::query("UPDATE agent_instance_volume_leases SET expires_at=now()+interval '1 hour',heartbeat_at=now() WHERE id=$1").bind(lease.id.as_uuid()).execute(&mut *tx).await.is_err());
    tx.rollback().await.unwrap();
    assert_eq!(fixture::evidence(&pool, f.volume).await, before);
}
