use sqlx::PgPool;
use uuid::Uuid;
use volume_trait::{
    OwnedBackingObservation, OwnedBackingPhase, VolumeError, VolumeOwnedProvisioningRepository,
};

use super::support::{begin, observation, revoke, setup};

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and published checkpoint 111"]
async fn concurrent_ready_observations_have_one_winner_without_late_resurrection(pool: PgPool) {
    let fixture = setup(&pool).await;
    let claim = begin(&fixture).await;
    let left = observation(&claim, OwnedBackingPhase::Ready);
    let right = observation(&claim, OwnedBackingPhase::Ready);
    let (left, right) = tokio::join!(
        fixture.worker.record_owned_observation(&claim, &left),
        fixture.worker.record_owned_observation(&claim, &right)
    );
    assert_eq!(usize::from(left.is_ok()) + usize::from(right.is_ok()), 1);
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM volume_owned_backing_observations WHERE phase='ready'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(count, 1);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and published checkpoint 111"]
async fn worker_readonly_reconciliation_survives_actor_loss_without_new_format_grant(pool: PgPool) {
    let fixture = setup(&pool).await;
    let claim = begin(&fixture).await;
    revoke(&fixture).await;
    let observed = observation(&claim, OwnedBackingPhase::Ready);
    assert!(matches!(
        fixture
            .actor
            .record_owned_observation(&claim, &observed)
            .await,
        Err(VolumeError::PermissionDenied)
    ));
    let context = fixture
        .worker
        .record_owned_observation(&claim, &observed)
        .await
        .unwrap();
    assert_eq!(context.observation, Some(observed.clone()));
    assert_eq!(
        fixture
            .worker
            .owned_reconciliation_context(&claim)
            .await
            .unwrap(),
        context
    );
    assert_eq!(
        fixture
            .worker
            .record_owned_observation(&claim, &observed)
            .await
            .unwrap(),
        context
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and published checkpoint 111"]
async fn exact_claim_and_original_physical_birth_identity_are_required(pool: PgPool) {
    let fixture = setup(&pool).await;
    let claim = begin(&fixture).await;
    let recorded = observation(&claim, OwnedBackingPhase::Recorded);
    fixture
        .worker
        .record_owned_observation(&claim, &recorded)
        .await
        .unwrap();
    let mut forged = claim.clone();
    forged.event_id = Uuid::new_v4();
    assert!(matches!(
        fixture.worker.owned_reconciliation_context(&forged).await,
        Err(VolumeError::IntentConflict)
    ));
    let mut changed = observation(&claim, OwnedBackingPhase::Ready)
        .fields()
        .clone();
    changed.root_inode += 1;
    assert!(
        fixture
            .worker
            .record_owned_observation(&claim, &OwnedBackingObservation::try_from(changed).unwrap())
            .await
            .is_err()
    );
    let mut changed = recorded.fields().clone();
    changed.journal_hash = [8; 32];
    assert!(matches!(
        fixture
            .worker
            .record_owned_observation(&claim, &OwnedBackingObservation::try_from(changed).unwrap())
            .await,
        Err(VolumeError::IntentConflict)
    ));
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and published checkpoint 111"]
async fn durable_format_intent_blocks_false_never_formatted_retry(pool: PgPool) {
    let fixture = setup(&pool).await;
    let claim = begin(&fixture).await;
    fixture
        .worker
        .record_owned_observation(&claim, &observation(&claim, OwnedBackingPhase::Recorded))
        .await
        .unwrap();
    fixture
        .worker
        .record_owned_observation(
            &claim,
            &observation(&claim, OwnedBackingPhase::FormatIntent),
        )
        .await
        .unwrap();
    assert!(
        fixture
            .worker
            .record_owned_observation(
                &claim,
                &observation(&claim, OwnedBackingPhase::FailedBeforeFormat)
            )
            .await
            .is_err()
    );
    let context = fixture
        .worker
        .owned_reconciliation_context(&claim)
        .await
        .unwrap();
    assert_eq!(
        context.observation.unwrap().fields().phase,
        OwnedBackingPhase::FormatIntent
    );
}
