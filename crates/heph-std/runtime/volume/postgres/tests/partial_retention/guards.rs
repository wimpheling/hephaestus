use super::support::{begin, identity, partial, setup};
use authz_postgres::begin_actor_transaction;
use sqlx::PgPool;
use volume_trait::{OwnedPartialBirthPhase, VolumePartialRetentionRepository};

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private112 schema"]
async fn raw_actor_cannot_forge_receipt_or_retarget_physical_head(pool: PgPool) {
    let f = setup(&pool).await;
    let claim = begin(&f).await;
    let ctx = f.worker.partial_retention_context(&claim).await.unwrap();
    let proof = partial(&ctx, OwnedPartialBirthPhase::ClaimedEmpty);
    let receipt = f
        .worker
        .record_partial_retention_observation(&ctx, &proof)
        .await
        .unwrap();
    let actors = crate::seals::role_pool(&pool, false).await;
    let mut tx = begin_actor_transaction(&actors, &identity(f.owner))
        .await
        .unwrap();
    assert!(sqlx::query("INSERT INTO volume_owned_partial_receipts SELECT * FROM volume_owned_partial_receipts WHERE id=$1")
        .bind(receipt.id.as_uuid()).execute(&mut *tx).await.is_err());
    tx.rollback().await.unwrap();
    let mut tx = begin_actor_transaction(&actors, &identity(f.owner))
        .await
        .unwrap();
    assert!(
        sqlx::query(
            "UPDATE volume_owned_observation_heads SET version=version+1 WHERE volume_id=$1"
        )
        .bind(proof.fields().volume_id)
        .execute(&mut *tx)
        .await
        .is_err()
    );
    tx.rollback().await.unwrap();
    actors.close().await;
}
#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private112 schema"]
async fn raw_worker_cannot_reuse_historical_receipt_as_new_head_progress(pool: PgPool) {
    let f = setup(&pool).await;
    let claim = begin(&f).await;
    let ctx = f.worker.partial_retention_context(&claim).await.unwrap();
    let proof = partial(&ctx, OwnedPartialBirthPhase::ClaimedEmpty);
    f.worker
        .record_partial_retention_observation(&ctx, &proof)
        .await
        .unwrap();
    let workers = crate::seals::role_pool(&pool, true).await;
    assert!(
        sqlx::query(
            "UPDATE volume_owned_observation_heads SET version=version+1 WHERE volume_id=$1"
        )
        .bind(proof.fields().volume_id)
        .execute(&workers)
        .await
        .is_err()
    );
    assert_eq!(
        f.worker
            .partial_retention_context(&claim)
            .await
            .unwrap()
            .head
            .version,
        1
    );
    workers.close().await;
}
