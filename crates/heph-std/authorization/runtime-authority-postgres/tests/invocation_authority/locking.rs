use super::harness::{Harness, code};
use sqlx::PgPool;

#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private119 PostgreSQL/native Local prerequisites"]
async fn held_volume_grant_and_capability_issuer_fail_closed_without_partial_ceiling(pool: PgPool) {
    let mut h = Box::pin(Harness::new(&pool, 2)).await;
    h.add_optional_bindings().await;
    let volume = h.birth.physical.files[0].0;
    for query in [
        sqlx::query("SELECT 1 FROM agent_instance_state_volumes WHERE id=$1 FOR UPDATE"),
        sqlx::query(
            "SELECT 1 FROM agent_instance_volume_mount_grants WHERE volume_id=$1 FOR UPDATE",
        ),
    ] {
        let before = h.rows().await;
        let mut holder = pool.begin().await.unwrap();
        query.bind(volume).execute(&mut *holder).await.unwrap();
        let error = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            h.raw_writer(&h.birth.birth.worker, &h.identity),
        )
        .await
        .unwrap()
        .unwrap_err();
        assert_eq!(code(&error).as_deref(), Some("55P03"), "{error:?}");
        holder.rollback().await.unwrap();
        assert_eq!(h.rows().await, before);
    }
    let before = h.rows().await;
    let mut holder = pool.begin().await.unwrap();
    sqlx::query("SELECT 1 FROM users WHERE id=$1 FOR UPDATE")
        .bind(h.birth.birth.actor.user_id.as_uuid())
        .execute(&mut *holder)
        .await
        .unwrap();
    let error = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        h.raw_writer(&h.birth.birth.worker, &h.identity),
    )
    .await
    .unwrap()
    .unwrap_err();
    assert_eq!(code(&error).as_deref(), Some("55P03"), "{error:?}");
    holder.rollback().await.unwrap();
    assert_eq!(h.rows().await, before);
    h.issue().await;
    h.birth.physical.assert_unchanged();
}
