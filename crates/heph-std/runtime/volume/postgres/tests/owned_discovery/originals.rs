use sqlx::PgPool;
use volume_postgres::PostgresVolumeMetadataRepository;
use volume_trait::{
    OwnedBackingPhase, OwnedProvisioningDiscovery, VolumeOwnedProvisioningRepository,
};

use super::fixture::{assert_original, begin, revoke, setup, snapshot};
use crate::provisioning::observation;

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private checkpoint111"]
async fn positive_birth_without_any_admitted_operation_is_readonly_unadmitted(pool: PgPool) {
    let (fixture, expected, request) = setup(&pool).await;
    let before = snapshot(&pool, &expected).await;
    for _ in 0..2 {
        assert_eq!(
            fixture
                .worker
                .discover_owned_provisioning(&expected)
                .await
                .unwrap(),
            OwnedProvisioningDiscovery::Unadmitted(Box::new(request.purpose.receipt().clone()))
        );
    }
    assert_eq!(snapshot(&pool, &expected).await, before);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private checkpoint111"]
async fn pending_operation_is_original_after_restart_and_creator_withdrawal(pool: PgPool) {
    let (fixture, expected, request) = setup(&pool).await;
    let original = begin(&fixture, &request).await;
    revoke(&fixture).await;
    let before = snapshot(&pool, &expected).await;
    let restarted =
        PostgresVolumeMetadataRepository::new(crate::seals::role_pool(&pool, true).await);
    assert_eq!(
        restarted
            .discover_owned_provisioning(&expected)
            .await
            .unwrap(),
        OwnedProvisioningDiscovery::Original(Box::new(original.clone()))
    );
    assert_original(&fixture, &expected, &original).await;
    assert_eq!(original.claim.operation_id, expected.first_operation_id());
    assert_eq!(original.claim.generation, 1);
    assert_eq!(snapshot(&pool, &expected).await, before);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private checkpoint111"]
async fn may_have_started_and_uncertain_progress_never_allocate_another_operation(pool: PgPool) {
    let (fixture, expected, request) = setup(&pool).await;
    let original = begin(&fixture, &request).await;
    for phase in [
        OwnedBackingPhase::Recorded,
        OwnedBackingPhase::FormatIntent,
        OwnedBackingPhase::Uncertain,
    ] {
        let progress = fixture
            .worker
            .record_owned_observation(&original.claim, &observation(&original.claim, phase))
            .await
            .unwrap();
        let before = snapshot(&pool, &expected).await;
        assert_original(&fixture, &expected, &progress).await;
        assert_eq!(snapshot(&pool, &expected).await, before);
        assert_eq!(progress.claim, original.claim);
    }
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private checkpoint111"]
async fn ready_returns_exact_original_receipt_and_observation_after_worker_restart(pool: PgPool) {
    let (fixture, expected, request) = setup(&pool).await;
    let original = begin(&fixture, &request).await;
    let progress = fixture
        .worker
        .record_owned_observation(
            &original.claim,
            &observation(&original.claim, OwnedBackingPhase::Ready),
        )
        .await
        .unwrap();
    revoke(&fixture).await;
    let before = snapshot(&pool, &expected).await;
    let restarted =
        PostgresVolumeMetadataRepository::new(crate::seals::role_pool(&pool, true).await);
    assert_eq!(
        restarted
            .discover_owned_provisioning(&expected)
            .await
            .unwrap(),
        OwnedProvisioningDiscovery::Original(Box::new(progress))
    );
    assert_eq!(snapshot(&pool, &expected).await, before);
}
