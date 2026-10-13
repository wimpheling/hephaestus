use super::support::{identity, intent, revoke, setup};
use sqlx::PgPool;
use uuid::Uuid;
use volume_trait::{
    OwnedVolumeRegistration, VolumeCreationIdentity, VolumeCreationLineage, VolumeError,
    VolumeOwnedRegistrationRepository, VolumeResourceRepository,
};

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL through DATABASE_URL and published checkpoint 110"]
async fn exact_replay_preserves_original_committed_receipt(pool: PgPool) {
    let fixture = setup(&pool).await;
    let intent = intent(&fixture);
    let first_identity = identity(fixture.owner);
    let first = fixture
        .actor
        .register_owned(&first_identity, &intent)
        .await
        .expect("created");
    assert_eq!(first.first_request_id, first_identity.request_id.as_uuid());
    assert_eq!(first.intent, intent);
    let replay = fixture
        .actor
        .register_owned(&identity(fixture.owner), &intent)
        .await
        .expect("replay");
    assert_eq!(first, replay);
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM application_events WHERE occurrence_id=$1")
            .bind(intent.creation().fields().operation_id.as_uuid())
            .fetch_one(&pool)
            .await
            .expect("committed event count");
    assert_eq!(count, 1);
    let stored_hash: Vec<u8> = sqlx::query_scalar("SELECT sha256($1::bytea)")
        .bind(intent.canonical_bytes())
        .fetch_one(&pool)
        .await
        .expect("fingerprint");
    assert_eq!(first.registration_hash.as_slice(), stored_hash);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL through DATABASE_URL and published checkpoint 110"]
async fn existing_independent_metadata_is_never_adopted(pool: PgPool) {
    let fixture = setup(&pool).await;
    let intent = intent(&fixture);
    fixture
        .actor
        .register(&identity(fixture.owner), intent.registration())
        .await
        .expect("independent registration");
    assert!(matches!(
        fixture
            .actor
            .register_owned(&identity(fixture.owner), &intent)
            .await,
        Err(VolumeError::IntentConflict)
    ));
    assert_eq!(
        fixture
            .worker
            .creation_lineage(intent.registration().id())
            .await
            .expect("worker inspection"),
        VolumeCreationLineage::Independent
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM volume_creation_seals")
        .fetch_one(&pool)
        .await
        .expect("no seals");
    assert_eq!(count, 0);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL through DATABASE_URL and published checkpoint 110"]
async fn changed_correlation_and_other_operation_conflict(pool: PgPool) {
    let fixture = setup(&pool).await;
    let original = intent(&fixture);
    fixture
        .actor
        .register_owned(&identity(fixture.owner), &original)
        .await
        .expect("created");
    let mut changed = original.creation().fields().clone();
    changed.input_hash = [9; 32];
    let changed = OwnedVolumeRegistration::new(
        original.registration().clone(),
        VolumeCreationIdentity::try_from(changed).expect("changed fingerprint"),
    );
    assert!(matches!(
        fixture
            .actor
            .register_owned(&identity(fixture.owner), &changed)
            .await,
        Err(VolumeError::IntentConflict)
    ));
    let other = intent(&fixture);
    let other =
        OwnedVolumeRegistration::new(original.registration().clone(), other.creation().clone());
    assert!(matches!(
        fixture
            .actor
            .register_owned(&identity(fixture.owner), &other)
            .await,
        Err(VolumeError::IntentConflict)
    ));
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL through DATABASE_URL and published checkpoint 110"]
async fn concurrent_same_operation_has_one_original_receipt(pool: PgPool) {
    let fixture = setup(&pool).await;
    let intent = intent(&fixture);
    let first = identity(fixture.owner);
    let second = identity(fixture.owner);
    let (first, second) = tokio::join!(
        fixture.actor.register_owned(&first, &intent),
        fixture.actor.register_owned(&second, &intent)
    );
    assert_eq!(
        first.expect("first registration"),
        second.expect("exact concurrent replay")
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM volume_creation_seals")
        .fetch_one(&pool)
        .await
        .expect("seal count");
    assert_eq!(count, 1);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL through DATABASE_URL and published checkpoint 110"]
async fn competing_operations_cannot_own_the_same_volume(pool: PgPool) {
    let fixture = setup(&pool).await;
    let first = intent(&fixture);
    let other = intent(&fixture);
    let second =
        OwnedVolumeRegistration::new(first.registration().clone(), other.creation().clone());
    let first_identity = identity(fixture.owner);
    let second_identity = identity(fixture.owner);
    let (first, second) = tokio::join!(
        fixture.actor.register_owned(&first_identity, &first),
        fixture.actor.register_owned(&second_identity, &second)
    );
    assert!(matches!(
        (&first, &second),
        (Ok(_), Err(VolumeError::IntentConflict)) | (Err(VolumeError::IntentConflict), Ok(_))
    ));
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL through DATABASE_URL and published checkpoint 110"]
async fn revocation_denies_replay_but_current_manager_can_read_old_lineage(pool: PgPool) {
    let fixture = setup(&pool).await;
    let intent = intent(&fixture);
    let receipt = fixture
        .actor
        .register_owned(&identity(fixture.owner), &intent)
        .await
        .expect("created");
    revoke(&fixture).await;
    let denied = identity(fixture.owner);
    assert!(matches!(
        fixture.actor.register_owned(&denied, &intent).await,
        Err(VolumeError::PermissionDenied)
    ));
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM authorization_audit_events WHERE request_id=$1 AND decision='deny'",
    )
    .bind(denied.request_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("durable deny");
    assert_eq!(count, 1);
    let expected = VolumeCreationLineage::Owned(Box::new(receipt));
    assert_eq!(
        fixture
            .actor
            .inspect_creation(&identity(fixture.successor), intent.registration().id())
            .await
            .expect("current cleanup manager"),
        expected
    );
    assert_eq!(
        fixture
            .worker
            .creation_lineage(intent.registration().id())
            .await
            .expect("worker original lineage"),
        expected
    );
    assert!(matches!(
        fixture
            .actor
            .register_owned(&identity(fixture.successor), &intent)
            .await,
        Err(VolumeError::PermissionDenied)
    ));
    assert!(matches!(
        fixture
            .actor
            .creation_lineage(intent.registration().id())
            .await,
        Err(VolumeError::PermissionDenied)
    ));
    assert!(matches!(
        fixture
            .worker
            .register_owned(&identity(fixture.successor), &intent)
            .await,
        Err(VolumeError::PermissionDenied)
    ));
    assert_eq!(
        fixture
            .worker
            .creation_lineage(runtime_types::VolumeId::from_uuid(Uuid::new_v4()))
            .await
            .expect("missing metadata observation"),
        VolumeCreationLineage::Missing
    );
}
