use sqlx::PgPool;
use uuid::Uuid;
use volume_trait::{
    OwnedBackingPhase, OwnedBackingPurpose, VolumeError, VolumeOwnedProvisioningRepository,
    VolumeRootNamespaceId,
};

use super::support::{begin, identity, observation, request, revoke, setup};

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and published checkpoint 111"]
async fn same_operation_replay_keeps_original_provenance_and_changed_purpose_conflicts(
    pool: PgPool,
) {
    let fixture = setup(&pool).await;
    let request = request(&fixture).await;
    let original = fixture
        .actor
        .begin_owned_provisioning(&identity(fixture.owner), &request)
        .await
        .unwrap();
    let replay = fixture
        .actor
        .begin_owned_provisioning(&identity(fixture.owner), &request)
        .await
        .unwrap();
    assert_eq!(replay, original);
    let mut changed = request.clone();
    changed.purpose = OwnedBackingPurpose::new(
        request.purpose.receipt().clone(),
        String::from("owned-host"),
        request.purpose.root().to_path_buf(),
        VolumeRootNamespaceId::from_uuid(Uuid::new_v4()).unwrap(),
        1,
    )
    .unwrap();
    assert!(matches!(
        fixture
            .actor
            .begin_owned_provisioning(&identity(fixture.owner), &changed)
            .await,
        Err(VolumeError::IntentConflict)
    ));
    let mut stale = request;
    stale.operation_id = Uuid::new_v4();
    assert!(matches!(
        fixture
            .actor
            .begin_owned_provisioning(&identity(fixture.owner), &stale)
            .await,
        Err(VolumeError::StaleLease)
    ));
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and published checkpoint 111"]
async fn ambiguity_blocks_other_operation_and_only_positive_failure_allows_retry(pool: PgPool) {
    let fixture = setup(&pool).await;
    let request = request(&fixture).await;
    let claim = fixture
        .actor
        .begin_owned_provisioning(&identity(fixture.owner), &request)
        .await
        .unwrap()
        .claim;
    let mut retry = request;
    retry.operation_id = Uuid::new_v4();
    retry.expected_generation = 1;
    assert!(matches!(
        fixture
            .actor
            .begin_owned_provisioning(&identity(fixture.owner), &retry)
            .await,
        Err(VolumeError::ProvisioningUncertain(_))
    ));
    fixture
        .worker
        .record_owned_observation(
            &claim,
            &observation(&claim, OwnedBackingPhase::FailedBeforeFormat),
        )
        .await
        .unwrap();
    let next = fixture
        .actor
        .begin_owned_provisioning(&identity(fixture.owner), &retry)
        .await
        .unwrap();
    assert_eq!(next.claim.generation, 2);
    assert_eq!(next.claim.purpose.birth_generation(), 1);
    assert!(matches!(
        fixture.worker.owned_reconciliation_context(&claim).await,
        Err(VolumeError::StaleLease)
    ));
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and published checkpoint 111"]
async fn new_actor_cannot_borrow_creation_actor_and_revoked_actor_cannot_replay(pool: PgPool) {
    let fixture = setup(&pool).await;
    let request = request(&fixture).await;
    assert!(matches!(
        fixture
            .actor
            .begin_owned_provisioning(&identity(fixture.successor), &request)
            .await,
        Err(VolumeError::PermissionDenied)
    ));
    fixture
        .actor
        .begin_owned_provisioning(&identity(fixture.owner), &request)
        .await
        .unwrap();
    revoke(&fixture).await;
    assert!(matches!(
        fixture
            .actor
            .begin_owned_provisioning(&identity(fixture.owner), &request)
            .await,
        Err(VolumeError::PermissionDenied)
    ));
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and published checkpoint 111"]
async fn concurrent_same_operation_has_one_fence_and_one_original_receipt(pool: PgPool) {
    let fixture = setup(&pool).await;
    let request = request(&fixture).await;
    let actor = identity(fixture.owner);
    let (left, right) = tokio::join!(
        fixture.actor.begin_owned_provisioning(&actor, &request),
        fixture.actor.begin_owned_provisioning(&actor, &request)
    );
    assert_eq!(left.unwrap(), right.unwrap());
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM volume_owned_provisioning_operations")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(count, 1);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and published checkpoint 111"]
async fn ready_is_permanent_for_creation_and_format_operations(pool: PgPool) {
    let fixture = setup(&pool).await;
    let claim = begin(&fixture).await;
    fixture
        .worker
        .record_owned_observation(&claim, &observation(&claim, OwnedBackingPhase::Ready))
        .await
        .unwrap();
    let request = volume_trait::BeginOwnedProvisioning {
        purpose: claim.purpose,
        operation_id: Uuid::new_v4(),
        expected_generation: 1,
    };
    assert!(matches!(
        fixture
            .actor
            .begin_owned_provisioning(&identity(fixture.owner), &request)
            .await,
        Err(VolumeError::InvalidState(_))
    ));
}
