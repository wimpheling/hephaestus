use super::support::{begin, partial, revoke, setup};
use sqlx::PgPool;
use uuid::Uuid;
use volume_trait::{
    OwnedPartialBirthObservation, OwnedPartialBirthPhase, VolumeError,
    VolumePartialRetentionRepository,
};

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private112 schema"]
async fn worker_records_one_exact_receipt_without_state_or_creation_generation_changes(
    pool: PgPool,
) {
    let f = setup(&pool).await;
    let claim = begin(&f).await;
    let ctx = f.worker.partial_retention_context(&claim).await.unwrap();
    assert_eq!(ctx.head.version, 0);
    let before: serde_json::Value = sqlx::query_scalar(
        "SELECT to_jsonb(volume) FROM agent_instance_state_volumes volume WHERE id=$1",
    )
    .bind(claim.purpose.receipt().intent.registration().id().as_uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    let proof = partial(&ctx, OwnedPartialBirthPhase::ClaimedEmpty);
    revoke(&f).await;
    let receipt = f
        .worker
        .record_partial_retention_observation(&ctx, &proof)
        .await
        .unwrap();
    assert_eq!(
        f.worker
            .record_partial_retention_observation(&ctx, &proof)
            .await
            .unwrap(),
        receipt
    );
    let head = f
        .worker
        .partial_retention_context(&claim)
        .await
        .unwrap()
        .head;
    assert_eq!(head.version, 1);
    assert_eq!(head.observation_id, Some(proof.fields().id));
    assert_eq!(head.partial_receipt_id, Some(receipt.id));
    let after: serde_json::Value = sqlx::query_scalar(
        "SELECT to_jsonb(volume) FROM agent_instance_state_volumes volume WHERE id=$1",
    )
    .bind(proof.fields().volume_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(before, after);
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM application_events WHERE id=$1 AND occurrence_id=$2",
    )
    .bind(receipt.event_id)
    .bind(proof.fields().id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(count, 1);
}
#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private112 schema"]
async fn actor_and_forged_original_claim_cannot_record_worker_receipts(pool: PgPool) {
    let f = setup(&pool).await;
    let claim = begin(&f).await;
    assert!(matches!(
        f.actor.partial_retention_context(&claim).await,
        Err(VolumeError::PermissionDenied)
    ));
    let ctx = f.worker.partial_retention_context(&claim).await.unwrap();
    let proof = partial(&ctx, OwnedPartialBirthPhase::AllocatedNeverFormat);
    assert!(matches!(
        f.actor
            .record_partial_retention_observation(&ctx, &proof)
            .await,
        Err(VolumeError::PermissionDenied)
    ));
    let mut forged = claim;
    forged.event_id = Uuid::new_v4();
    assert!(f.worker.partial_retention_context(&forged).await.is_err());
    let mut changed = proof.fields().clone();
    changed.original_creation.actor_id = Uuid::new_v4();
    assert!(matches!(
        f.worker
            .record_partial_retention_observation(
                &ctx,
                &OwnedPartialBirthObservation::try_from(changed).unwrap()
            )
            .await,
        Err(VolumeError::IntentConflict)
    ));
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM volume_owned_partial_receipts")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
}
#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private112 schema"]
async fn simultaneous_worker_receipts_have_one_head_cas_winner(pool: PgPool) {
    let f = setup(&pool).await;
    let claim = begin(&f).await;
    let ctx = f.worker.partial_retention_context(&claim).await.unwrap();
    let left = partial(&ctx, OwnedPartialBirthPhase::AllocatedNeverFormat);
    let right = partial(&ctx, OwnedPartialBirthPhase::AllocatedNeverFormat);
    let (left, right) = tokio::join!(
        f.worker.record_partial_retention_observation(&ctx, &left),
        f.worker.record_partial_retention_observation(&ctx, &right)
    );
    assert_eq!(usize::from(left.is_ok()) + usize::from(right.is_ok()), 1);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM volume_owned_partial_receipts")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
    assert_eq!(
        f.worker
            .partial_retention_context(&claim)
            .await
            .unwrap()
            .head
            .version,
        1
    );
}
