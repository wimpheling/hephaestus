use super::harness::{Harness, pg_time};
use runtime_authority::{LiveRunSnapshotResolver, RuntimeSessionRepository};
use sqlx::PgPool;
use volume_trait::RunVolumeMetadataRepository;

#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private119 PostgreSQL/native Local prerequisites"]
async fn exact_typed_mount_fences_and_current_grant_remain_authoritative_after_issuance(
    pool: PgPool,
) {
    let h = Box::pin(Harness::new(&pool, 2)).await;
    let selections = h
        .birth
        .volumes()
        .load_run_selections(h.birth.start.run_id)
        .await
        .unwrap();
    let scope = selections.selections()[0].scope();
    assert!(
        h.repository
            .volume_mount_authorized(h.birth.start.run_id, scope)
            .await
            .unwrap()
    );
    let stored = h.issue().await;
    assert!(
        h.repository
            .volume_mount_authorized(h.birth.start.run_id, scope)
            .await
            .unwrap()
    );
    let now = pg_time();
    h.birth
        .volumes()
        .heartbeat_run(
            &selections,
            h.birth.scope.host_id(),
            now,
            now + time::Duration::minutes(10),
        )
        .await
        .unwrap();
    let before = h.rows().await;
    assert!(
        h.repository
            .resolve_live_snapshot(
                &h.ceiling(&stored),
                h.snapshot.authorization_model_version(),
                now
            )
            .await
            .is_err(),
        "existing ceiling outlives a current fence expiry"
    );
    assert!(h.repository.find(stored.id).await.is_err());
    assert_eq!(h.rows().await, before);
    h.birth.physical.assert_unchanged();
}
#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private119 PostgreSQL/native Local prerequisites"]
async fn actual_mount_revocation_blocks_mount_and_pending_ceiling_without_fence_release(
    pool: PgPool,
) {
    let h = Box::pin(Harness::new(&pool, 2)).await;
    let selections = h
        .birth
        .volumes()
        .load_run_selections(h.birth.start.run_id)
        .await
        .unwrap();
    let scope = selections.selections()[0].scope();
    let stored = h.issue().await;
    let mut actor =
        authz_postgres::begin_actor_transaction(&h.birth.birth.app, &h.birth.birth.actor)
            .await
            .unwrap();
    sqlx::query("INSERT INTO agent_instance_volume_mount_revocations(id,grant_id,created_by,request_id,authorization_model_version) SELECT $1,id,$2,$5,$6 FROM agent_instance_volume_mount_grants WHERE instance_revision_id=$3 AND volume_id=$4")
  .bind(uuid::Uuid::new_v4()).bind(h.birth.birth.actor.user_id.as_uuid()).bind(h.birth.start.instance_revision_id.as_uuid()).bind(scope.volume_id().as_uuid()).bind(h.birth.birth.actor.request_id.as_uuid()).bind(h.snapshot.authorization_model_version())
  .execute(&mut *actor).await.unwrap();
    actor.commit().await.unwrap();
    let before = h.rows().await;
    assert!(
        !h.repository
            .volume_mount_authorized(h.birth.start.run_id, scope)
            .await
            .unwrap()
    );
    assert!(h.repository.find(stored.id).await.is_err());
    assert!(
        h.repository
            .resolve_live_snapshot(
                &h.ceiling(&stored),
                h.snapshot.authorization_model_version(),
                pg_time()
            )
            .await
            .is_err()
    );
    assert_eq!(h.rows().await, before);
    h.birth.physical.assert_unchanged();
}
