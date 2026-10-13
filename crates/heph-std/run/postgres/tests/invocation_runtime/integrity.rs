use super::{fixture::Fixture, locking::code};
use run_domain::RunState;
use run_orchestrator::RunRepository;
use sqlx::PgPool;
use volume_trait::RunVolumeMetadataRepository;

#[sqlx::test(migrations = false)]
#[ignore = "requires actual private119 PostgreSQL/native ext4"]
async fn actor_raw_acquisition_and_snapshot_forgery_cannot_establish_worker_fences(pool: PgPool) {
    let h = Fixture::new(&pool, 2).await;
    h.consume().await.unwrap();
    h.repository()
        .transition(h.start.run_id, RunState::LeasingVolume, None, None)
        .await
        .unwrap();
    let selected = h
        .volumes()
        .load_run_selections(h.start.run_id)
        .await
        .unwrap();
    let before = h.rows().await;
    let mut actor = authz_postgres::begin_actor_transaction(&h.birth.app, &h.birth.actor)
        .await
        .unwrap();
    let now = time::OffsetDateTime::now_utc();
    let error = sqlx::query("SELECT acquire_qualified_invocation119($1,$2,$3,$4,$5,$6)")
        .bind(h.start.run_id.as_uuid())
        .bind(serde_json::to_vec(&h.config.canonical_json()).unwrap())
        .bind(serde_json::to_value(&selected).unwrap())
        .bind(h.scope.host_id())
        .bind(now)
        .bind(now + time::Duration::minutes(5))
        .execute(&mut *actor)
        .await
        .unwrap_err();
    assert_eq!(code(&error).as_deref(), Some("42501"), "{error:?}");
    actor.rollback().await.unwrap();
    assert_eq!(h.rows().await, before);
    let mut raw = h.birth.worker.begin().await.unwrap();
    let result = sqlx::query("UPDATE runs SET volume_selection_snapshot='{}'::jsonb WHERE id=$1")
        .bind(h.start.run_id.as_uuid())
        .execute(&mut *raw)
        .await;
    assert!(result.is_err());
    raw.rollback().await.unwrap();
    assert_eq!(h.rows().await, before);
    h.physical.assert_unchanged();
}

#[sqlx::test(migrations = false)]
#[ignore = "requires actual private119 PostgreSQL/native ext4"]
async fn expired_own_set_stays_held_without_new_generation_or_renewal(pool: PgPool) {
    let h = Fixture::new(&pool, 2).await;
    h.consume().await.unwrap();
    h.repository()
        .transition(h.start.run_id, RunState::LeasingVolume, None, None)
        .await
        .unwrap();
    let selected = h
        .volumes()
        .load_run_selections(h.start.run_id)
        .await
        .unwrap();
    let now = time::OffsetDateTime::now_utc();
    let expiry = now + time::Duration::seconds(2);
    let first = h
        .volumes()
        .acquire_run(&selected, h.scope.host_id(), now, expiry)
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let past: bool = sqlx::query_scalar("SELECT clock_timestamp()>$1")
                .bind(expiry)
                .fetch_one(&pool)
                .await
                .unwrap();
            if past {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let before = h.rows().await;
    let later = time::OffsetDateTime::now_utc();
    assert!(
        h.volumes()
            .acquire_run(
                &selected,
                h.scope.host_id(),
                later,
                later + time::Duration::minutes(5)
            )
            .await
            .is_err()
    );
    assert!(
        h.volumes()
            .heartbeat_run(
                &selected,
                h.scope.host_id(),
                later,
                later + time::Duration::minutes(5)
            )
            .await
            .is_err()
    );
    assert_eq!(
        h.volumes()
            .leases_for_run(h.start.run_id, h.scope.host_id())
            .await
            .unwrap(),
        first
    );
    assert_eq!(h.rows().await, before);
    h.physical.assert_unchanged();
}

#[sqlx::test(migrations = false)]
#[ignore = "requires actual private119 PostgreSQL/native ext4"]
async fn immutable_fence_and_generation_tampering_is_denied_with_exact_unchanged_graph(
    pool: PgPool,
) {
    let h = Fixture::new(&pool, 2).await;
    h.consume().await.unwrap();
    h.repository()
        .transition(h.start.run_id, RunState::LeasingVolume, None, None)
        .await
        .unwrap();
    let selected = h
        .volumes()
        .load_run_selections(h.start.run_id)
        .await
        .unwrap();
    let now = time::OffsetDateTime::now_utc();
    let first = h
        .volumes()
        .acquire_run(
            &selected,
            h.scope.host_id(),
            now,
            now + time::Duration::minutes(5),
        )
        .await
        .unwrap();
    let before = h.rows().await;
    let mut raw = h.birth.worker.begin().await.unwrap();
    assert!(
        sqlx::query(
            "UPDATE agent_instance_volume_leases SET fencing_token=fencing_token+1 WHERE id=$1"
        )
        .bind(first[0].lease().id.as_uuid())
        .execute(&mut *raw)
        .await
        .is_err()
    );
    raw.rollback().await.unwrap();
    let mut raw = h.birth.worker.begin().await.unwrap();
    let update = sqlx::query(
        "UPDATE agent_instance_state_volumes SET lease_generation=lease_generation+1 WHERE id=$1",
    )
    .bind(first[0].lease().volume_id.as_uuid())
    .execute(&mut *raw)
    .await;
    if update.is_ok() {
        assert!(
            raw.commit().await.is_err(),
            "deferred full-set CAS rejects inconsistent generation"
        );
    } else {
        raw.rollback().await.unwrap();
    }
    assert_eq!(h.rows().await, before);
    h.physical.assert_unchanged();
}
