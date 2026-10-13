use runtime_types::{RunId, VolumeId};
use sqlx::PgPool;
use uuid::Uuid;
use volume_postgres::{PostgresVolumeMetadataRepository, ScalarMetadataSchema};
use volume_trait::VolumeMetadataRepository;

use super::{fixture, producers};

#[sqlx::test(migrations = false)]
#[ignore = "requires actual disposable private scalar PostgreSQL"]
async fn actor_role_cannot_call_scalar_helpers(pool: PgPool) {
    let (f, e, _, lease) = fixture::acquired(&pool).await;
    let before = fixture::evidence(&pool, f.volume).await;
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SET LOCAL ROLE hephaestus_app")
        .execute(&mut *tx)
        .await
        .unwrap();
    let error = sqlx::query("SELECT legacy_scalar_active($1,'host')")
        .bind(e.run)
        .execute(&mut *tx)
        .await
        .unwrap_err();
    assert_eq!(
        error
            .as_database_error()
            .and_then(sqlx::error::DatabaseError::code)
            .as_deref(),
        Some("42501")
    );
    tx.rollback().await.unwrap();
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('hephaestus.actor_id',$1,true),set_config('hephaestus.request_id',$2,true)")
        .bind(f.maintainer.to_string()).bind(Uuid::new_v4().to_string()).execute(&mut *tx).await.unwrap();
    sqlx::query("SET LOCAL ROLE hephaestus_app")
        .execute(&mut *tx)
        .await
        .unwrap();
    let visible: i64 =
        sqlx::query_scalar("SELECT count(*) FROM agent_instance_volume_leases WHERE id=$1")
            .bind(lease.id.as_uuid())
            .fetch_one(&mut *tx)
            .await
            .unwrap();
    assert_eq!(
        visible, 1,
        "raw denial must reach a real actor-visible lease"
    );
    let may_update: i32 = sqlx::query_scalar(
        "SELECT check_permission('user',hephaestus_actor_id(),'can_update','agent_instance',$1)",
    )
    .bind(f.instance.to_string())
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    assert_eq!(
        may_update, 1,
        "fixture actor passes the ordinary lease write policy"
    );
    let error=sqlx::query("UPDATE agent_instance_volume_leases SET heartbeat_at=now()+interval '1 millisecond' WHERE id=$1")
        .bind(lease.id.as_uuid()).execute(&mut *tx).await.unwrap_err();
    assert_eq!(
        error
            .as_database_error()
            .and_then(sqlx::error::DatabaseError::code)
            .as_deref(),
        Some("42501")
    );
    tx.rollback().await.unwrap();
    let ordinary = PostgresVolumeMetadataRepository::new(pool.clone())
        .with_scalar_schema(ScalarMetadataSchema::Profiled);
    assert!(
        ordinary.begin_recovery(&lease).await.is_err(),
        "admin connection cannot impersonate worker through helper"
    );
    assert_eq!(fixture::evidence(&pool, f.volume).await, before);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires actual disposable private scalar PostgreSQL"]
async fn fresh_lease_and_run_binding_commit_atomically_and_binding_never_changes(pool: PgPool) {
    let f = fixture::setup(&pool).await;
    let e = fixture::Execution::normal(&pool, &f).await;
    producers::normal(&pool, &e, "legacy_scalar", false)
        .await
        .unwrap();
    fixture::advance(&pool, e.run, "leasing_volume").await;
    let before = fixture::evidence(&pool, f.volume).await;
    let mut tx = fixture::worker(&pool).await;
    sqlx::query("INSERT INTO agent_instance_volume_leases(id,volume_id,instance_id,run_id,host_id,fencing_token,state,expires_at) VALUES($1,$2,$3,$4,'host',1,'active',now()+interval '1 hour')")
        .bind(Uuid::new_v4()).bind(f.volume).bind(e.instance).bind(e.run).execute(&mut *tx).await.unwrap();
    assert!(
        tx.commit().await.is_err(),
        "positive lease cannot commit the acquire-before-binding crash gap"
    );
    assert_eq!(fixture::evidence(&pool, f.volume).await, before);
    let repository = fixture::repository(&pool).await;
    let (now, expires) = fixture::times();
    let lease = repository
        .acquire(
            VolumeId::from_uuid(f.volume),
            RunId::from_uuid(e.run),
            "host",
            now,
            expires,
        )
        .await
        .unwrap();
    assert_eq!(
        lease.fencing_token, 1,
        "failed birth transaction did not advance the resource generation"
    );
    let mut tx = fixture::worker(&pool).await;
    assert!(
        sqlx::query(
            "UPDATE runs SET volume_id=NULL,lease_id=NULL,lease_fencing_token=NULL WHERE id=$1"
        )
        .bind(e.run)
        .execute(&mut *tx)
        .await
        .is_err()
    );
    tx.rollback().await.unwrap();
    let binding: (Uuid, Uuid, i64, Option<String>) =
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
async fn lease_only_release_rolls_back_and_held_ready_reset_is_denied(pool: PgPool) {
    let (f, e, repository, lease) = fixture::acquired(&pool).await;
    let (now, expires) = fixture::times();
    repository
        .mark_attached(&lease, now, expires)
        .await
        .unwrap();
    fixture::advance(&pool, e.run, "cleaning_up").await;
    let before = fixture::evidence(&pool, f.volume).await;
    let mut tx = fixture::worker(&pool).await;
    sqlx::query(
        "UPDATE agent_instance_volume_leases SET released_at=now(),state='released' WHERE id=$1",
    )
    .bind(lease.id.as_uuid())
    .execute(&mut *tx)
    .await
    .unwrap();
    assert!(
        tx.commit().await.is_err(),
        "lease-only change must fail deferred same-fence Ready proof"
    );
    assert_eq!(fixture::evidence(&pool, f.volume).await, before);
    let mut tx = fixture::worker(&pool).await;
    assert!(
        sqlx::query("UPDATE agent_instance_state_volumes SET state='ready' WHERE id=$1")
            .bind(f.volume)
            .execute(&mut *tx)
            .await
            .is_err()
    );
    tx.rollback().await.unwrap();
    assert_eq!(fixture::evidence(&pool, f.volume).await, before);
    repository
        .release_after_detach(&lease, false)
        .await
        .unwrap();
}

#[sqlx::test(migrations = false)]
#[ignore = "requires actual disposable private scalar PostgreSQL"]
async fn owned_canonical_and_historical_runs_cannot_use_scalar_projection(pool: PgPool) {
    let f = fixture::setup(&pool).await;
    let e = fixture::Execution::normal(&pool, &f).await;
    producers::normal(&pool, &e, "owned_canonical", true)
        .await
        .unwrap();
    fixture::advance(&pool, e.run, "leasing_volume").await;
    let repository = fixture::repository(&pool).await;
    let before = fixture::evidence(&pool, f.volume).await;
    let (now, expires) = fixture::times();
    assert!(
        repository
            .acquire(
                VolumeId::from_uuid(f.volume),
                RunId::from_uuid(e.run),
                "host",
                now,
                expires
            )
            .await
            .is_err()
    );
    assert!(
        repository
            .active_lease_for_run(RunId::from_uuid(e.run), "host")
            .await
            .is_err()
    );
    assert!(
        repository
            .active_lease_for_run(RunId::from_uuid(f.run), "host")
            .await
            .is_err()
    );
    assert_eq!(fixture::evidence(&pool, f.volume).await, before);
    let mut tx = fixture::worker(&pool).await;
    sqlx::query("INSERT INTO agent_instance_volume_leases(id,volume_id,instance_id,run_id,host_id,fencing_token,state,expires_at) VALUES($1,$2,$3,$4,'host',1,'active',now()+interval '1 hour')")
        .bind(Uuid::new_v4()).bind(f.volume).bind(e.instance).bind(e.run).execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    fixture::advance(&pool, e.run, "cleaning_up").await;
    let before = fixture::evidence(&pool, f.volume).await;
    let mut tx = fixture::worker(&pool).await;
    assert!(sqlx::query("UPDATE agent_instance_volume_leases SET state='released',released_at=now() WHERE run_id=$1").bind(e.run).execute(&mut *tx).await.is_err());
    tx.rollback().await.unwrap();
    assert_eq!(
        fixture::evidence(&pool, f.volume).await,
        before,
        "canonical trusted receipt guard remains strict"
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires actual disposable private scalar PostgreSQL"]
async fn global_historical_lease_survives_upgrade_and_blocks_new_scalar_acquisition(pool: PgPool) {
    let f = fixture::baseline(&pool).await;
    let historical = Uuid::new_v4();
    sqlx::query("INSERT INTO agent_instance_volume_leases(id,volume_id,instance_id,run_id,host_id,fencing_token,state,expires_at) VALUES($1,$2,$3,$4,'host',7,'active',now()+interval '1 hour')")
        .bind(historical).bind(f.volume).bind(f.instance).bind(f.run).execute(&pool).await.unwrap();
    sqlx::query("UPDATE agent_instance_state_volumes SET lease_generation=7 WHERE id=$1")
        .bind(f.volume)
        .execute(&pool)
        .await
        .unwrap();
    let checksums: Vec<(i64, Vec<u8>)> =
        sqlx::query_as("SELECT version,checksum FROM _sqlx_migrations ORDER BY version")
            .fetch_all(&pool)
            .await
            .unwrap();
    fixture::checkpoint(108).run(&pool).await.unwrap();
    let after: Vec<(i64, Vec<u8>)> = sqlx::query_as(
        "SELECT version,checksum FROM _sqlx_migrations WHERE version<=107 ORDER BY version",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(checksums, after);
    let before = fixture::evidence(&pool, f.volume).await;
    let e = fixture::Execution::normal(&pool, &f).await;
    producers::normal(&pool, &e, "legacy_scalar", false)
        .await
        .unwrap();
    fixture::advance(&pool, e.run, "leasing_volume").await;
    let repository = fixture::repository(&pool).await;
    let (now, expires) = fixture::times();
    assert!(
        repository
            .acquire(
                VolumeId::from_uuid(f.volume),
                RunId::from_uuid(e.run),
                "host",
                now,
                expires
            )
            .await
            .is_err()
    );
    assert!(
        repository
            .stale_leases(expires + time::Duration::hours(2))
            .await
            .unwrap()
            .is_empty()
    );
    let mut tx = fixture::worker(&pool).await;
    assert!(
        sqlx::query(
            "UPDATE agent_instance_volume_leases SET state='released',released_at=now() WHERE id=$1"
        )
        .bind(historical)
        .execute(&mut *tx)
        .await
        .is_err()
    );
    tx.rollback().await.unwrap();
    assert_eq!(
        fixture::evidence(&pool, f.volume).await,
        before,
        "backfilled origin is not positive fresh execution"
    );
}
