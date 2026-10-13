use super::support::{begin, observation, partial, revoke, setup};
use sqlx::PgPool;
use volume_trait::{
    OwnedBackingPhase, OwnedPartialBirthObservation, OwnedPartialBirthPhase,
    VolumeOwnedProvisioningRepository, VolumePartialRetentionRepository,
};

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private112 schema"]
async fn original_formatter_progress_invalidates_old_partial_receipt_and_head(pool: PgPool) {
    let f = setup(&pool).await;
    let claim = begin(&f).await;
    // A partial receipt observes physical facts without changing provisioning
    // state. The original formatter needs the legal 111 allocation observation.
    f.worker
        .record_owned_observation(&claim, &observation(&claim, OwnedBackingPhase::Recorded))
        .await
        .unwrap();
    let ctx = f.worker.partial_retention_context(&claim).await.unwrap();
    let proof = partial(&ctx, OwnedPartialBirthPhase::AllocatedNeverFormat);
    f.worker
        .record_partial_retention_observation(&ctx, &proof)
        .await
        .unwrap();
    let intent = observation(&claim, OwnedBackingPhase::FormatIntent);
    f.worker
        .record_owned_observation(&claim, &intent)
        .await
        .unwrap();
    let head = f
        .worker
        .partial_retention_context(&claim)
        .await
        .unwrap()
        .head;
    assert_eq!(head.version, 3);
    assert_eq!(head.observation_id, Some(intent.fields().id));
    assert_eq!(head.backing_phase, Some(OwnedBackingPhase::FormatIntent));
    assert!(
        f.worker
            .record_partial_retention_observation(&ctx, &proof)
            .await
            .is_err()
    );
}
#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private112 schema"]
async fn known_ready_worker_context_rejects_partial_even_after_actor_loss(pool: PgPool) {
    let f = setup(&pool).await;
    let claim = begin(&f).await;
    let ctx = f.worker.partial_retention_context(&claim).await.unwrap();
    let proof = partial(&ctx, OwnedPartialBirthPhase::FormatIntentIncomplete);
    f.worker
        .record_owned_observation(&claim, &observation(&claim, OwnedBackingPhase::Ready))
        .await
        .unwrap();
    revoke(&f).await;
    assert!(f.worker.partial_retention_context(&claim).await.is_err());
    assert!(
        f.worker
            .record_partial_retention_observation(&ctx, &proof)
            .await
            .is_err()
    );
}
#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private112 schema"]
async fn rejected_birth_identity_rolls_back_worker_receipt_head_and_event(pool: PgPool) {
    let f = setup(&pool).await;
    let claim = begin(&f).await;
    f.worker
        .record_owned_observation(&claim, &observation(&claim, OwnedBackingPhase::Recorded))
        .await
        .unwrap();
    let ctx = f.worker.partial_retention_context(&claim).await.unwrap();
    let mut wire = partial(&ctx, OwnedPartialBirthPhase::AllocatedNeverFormat)
        .fields()
        .clone();
    wire.birth_record_hash = [99; 32];
    let proof = OwnedPartialBirthObservation::try_from(wire).unwrap();
    let events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM application_events WHERE scope_kind='project' AND scope_id=$1",
    )
    .bind(f.project)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        f.worker
            .record_partial_retention_observation(&ctx, &proof)
            .await
            .is_err()
    );
    assert_eq!(
        f.worker.partial_retention_context(&claim).await.unwrap(),
        ctx
    );
    let after: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM application_events WHERE scope_kind='project' AND scope_id=$1",
    )
    .bind(f.project)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(events, after);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM volume_owned_partial_receipts")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
}
