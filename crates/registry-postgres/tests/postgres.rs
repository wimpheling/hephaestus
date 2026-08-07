//! Opt-in integration coverage for the registry control-plane migration.

use builder_catalog_domain::OciImageId;
use registry_domain::{
    ImmutableManifestReference, NamespaceClaim, OciDescriptor, OciMediaType, PlatformDescriptor,
    PlatformImageKey, PolicyVersion, PublicationIntent, PublicationIntentId, RegistryAuthority,
    RegistryOwner, Sha256Digest, SupplyChainEvidence, SupplyChainPolicy, SupplyChainReferrer,
    SupplyChainReferrerKind, VerifiedPublication,
};
use registry_postgres::{
    NewRegistryNotification, NotificationCompletion, PgRegistryStore, RegistryNotificationAction,
    RegistryNotificationTarget,
};
use serial_test::serial;
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::env;
use time::OffsetDateTime;

#[tokio::test]
#[serial]
async fn migration_enforces_registry_lifecycle_and_outbox_atomicity() {
    let Some(pool) = fixture().await else {
        return;
    };
    let store = PgRegistryStore::new(pool.clone());
    let intent = intent();
    let created = store.create_intent(&intent).await.expect("create intent");
    assert_eq!(
        store
            .list_for_namespace(created.reference().namespace())
            .await
            .expect("namespace intents"),
        vec![created.clone()]
    );
    assert!(
        store
            .list_all()
            .await
            .expect("all registry intents")
            .iter()
            .any(|candidate| candidate.id() == created.id())
    );
    assert_platform_catalog_rls(&pool, created.id().as_uuid()).await;
    let owner_id = uuid::Uuid::new_v4();
    let outsider_id = uuid::Uuid::new_v4();
    let project_id = seed_project_reader(&pool, owner_id, outsider_id).await;
    let project_intent = project_intent(project_id, OciImageId::new());
    let project_publication = store
        .create_intent(&project_intent)
        .await
        .expect("create project publication");
    assert_project_registry_rls(
        &pool,
        owner_id,
        outsider_id,
        project_publication.id().as_uuid(),
    )
    .await;
    assert_eq!(
        store
            .create_intent(&intent)
            .await
            .expect("idempotent create"),
        created
    );

    let namespace_id: uuid::Uuid =
        sqlx::query_scalar("SELECT id FROM registry_namespaces WHERE repository_path = $1")
            .bind(created.reference().namespace().as_str())
            .fetch_one(&pool)
            .await
            .expect("namespace");
    assert!(
        sqlx::query(
            "UPDATE registry_namespaces SET repository_path = 'platform/images/other' WHERE id = $1"
        )
        .bind(namespace_id)
        .execute(&pool)
        .await
        .is_err()
    );
    assert!(
        sqlx::query(
            "INSERT INTO registry_publications (
            id, namespace_id, owner_kind, platform_image_key, registry_authority,
            expected_digest, expected_media_type, expected_size, policy_version
         ) VALUES (gen_random_uuid(), $1, 'platform_image', 'other', 'registry.example',
            $2, 'application/vnd.oci.image.index.v1+json', 100, 'v1')",
        )
        .bind(namespace_id)
        .bind(created.reference().digest().as_str())
        .execute(&pool)
        .await
        .is_err()
    );

    let verification = verification(&created);
    assert_eq!(
        store
            .record_verified(created.id(), verification.clone())
            .await
            .expect("verify")
            .state(),
        registry_domain::PublicationState::Verified
    );
    let concurrent_store = PgRegistryStore::new(pool.clone());
    let (left, right) = tokio::join!(
        store.approve(created.id()),
        concurrent_store.approve(created.id())
    );
    assert_eq!(
        left.expect("left approval").state(),
        registry_domain::PublicationState::Approved
    );
    assert_eq!(
        right.expect("right approval").state(),
        registry_domain::PublicationState::Approved
    );
    assert_eq!(
        store
            .create_intent(&intent)
            .await
            .expect("idempotent replay after state change")
            .state(),
        registry_domain::PublicationState::Approved
    );

    let event_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM application_events WHERE aggregate_type = 'registry_publication' AND aggregate_id = $1",
    )
    .bind(created.id().as_uuid())
    .fetch_one(&pool)
    .await
    .expect("lifecycle events");
    let outbox_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM product_event_outbox outbox
         JOIN application_events event ON event.id = outbox.event_id
         WHERE event.aggregate_id = $1",
    )
    .bind(created.id().as_uuid())
    .fetch_one(&pool)
    .await
    .expect("product outbox");
    assert_eq!(event_count, outbox_count);
    assert!(event_count >= 2);
    let typed_event_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM application_events
         WHERE aggregate_type = 'registry_publication' AND aggregate_id = $1
           AND event_type = 'registry.publication_changed'",
    )
    .bind(created.id().as_uuid())
    .fetch_one(&pool)
    .await
    .expect("typed registry product events");
    assert_eq!(typed_event_count, event_count);

    assert_eq!(
        store
            .mark_missing(created.id())
            .await
            .expect("missing")
            .state(),
        registry_domain::PublicationState::Missing
    );
    assert_eq!(
        store
            .restore_verified(created.id(), &verification)
            .await
            .expect("restore")
            .state(),
        registry_domain::PublicationState::Approved
    );
    assert_eq!(
        store.retire(created.id()).await.expect("retire").state(),
        registry_domain::PublicationState::Retired
    );
    assert!(store.mark_missing(created.id()).await.is_err());

    let notification = NewRegistryNotification {
        event_key: format!("zot-{}", uuid::Uuid::new_v4()),
        repository_path: created.reference().namespace().as_str().to_owned(),
        action: RegistryNotificationAction::Push,
        target: Some(RegistryNotificationTarget {
            digest: created.expected_manifest().digest().clone(),
            media_type: created.expected_manifest().media_type().clone(),
        }),
        occurred_at: OffsetDateTime::now_utc(),
        payload_sha256: [7; 32],
    };
    let first = store
        .ingest_notification(notification.clone())
        .await
        .expect("inbox insert");
    let duplicate = store
        .ingest_notification(notification)
        .await
        .expect("inbox dedupe");
    assert_eq!(first.id, duplicate.id);
    assert!(!first.duplicate);
    assert!(duplicate.duplicate);
    let claimed = store
        .claim_notification(std::time::Duration::from_secs(30))
        .await
        .expect("claim")
        .expect("notification available");
    store
        .complete_notification(
            claimed.id,
            claimed.claim_token,
            NotificationCompletion::Processed,
        )
        .await
        .expect("complete notification");

    let orphan = NewRegistryNotification {
        event_key: format!("zot-{}", uuid::Uuid::new_v4()),
        repository_path: String::from("orphan/imported/content"),
        action: RegistryNotificationAction::Delete,
        target: None,
        occurred_at: OffsetDateTime::now_utc(),
        payload_sha256: [8; 32],
    };
    store
        .ingest_notification(orphan)
        .await
        .expect("orphan observation");
    let claimed_orphan = store
        .claim_notification(std::time::Duration::from_secs(30))
        .await
        .expect("claim orphan")
        .expect("orphan available");
    assert_eq!(claimed_orphan.repository_path, "orphan/imported/content");
    assert!(claimed_orphan.namespace.is_none());
}

async fn assert_platform_catalog_rls(pool: &PgPool, publication_id: uuid::Uuid) {
    let mut authenticated = pool.begin().await.expect("authenticated transaction");
    sqlx::query(
        "SELECT set_config('hephaestus.actor_id', $1, true),
                set_config('hephaestus.subject_type', 'user', true)",
    )
    .bind(uuid::Uuid::new_v4().to_string())
    .execute(&mut *authenticated)
    .await
    .expect("set authenticated actor");
    sqlx::query("SET LOCAL ROLE hephaestus_app")
        .execute(&mut *authenticated)
        .await
        .expect("use application role");
    let visible =
        sqlx::query_scalar::<_, uuid::Uuid>("SELECT id FROM registry_publications WHERE id = $1")
            .bind(publication_id)
            .fetch_optional(&mut *authenticated)
            .await
            .expect("authenticated platform catalog read");
    assert_eq!(visible, Some(publication_id));
    authenticated
        .rollback()
        .await
        .expect("close authenticated transaction");

    let mut anonymous = pool.begin().await.expect("anonymous transaction");
    sqlx::query("SET LOCAL ROLE hephaestus_app")
        .execute(&mut *anonymous)
        .await
        .expect("use application role");
    let hidden =
        sqlx::query_scalar::<_, uuid::Uuid>("SELECT id FROM registry_publications WHERE id = $1")
            .bind(publication_id)
            .fetch_optional(&mut *anonymous)
            .await
            .expect("anonymous platform catalog read");
    assert!(hidden.is_none());
    anonymous
        .rollback()
        .await
        .expect("close anonymous transaction");
}

async fn seed_project_reader(
    pool: &PgPool,
    owner_id: uuid::Uuid,
    outsider_id: uuid::Uuid,
) -> uuid::Uuid {
    let organization_id = uuid::Uuid::new_v4();
    let project_id = uuid::Uuid::new_v4();
    sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, 'registry-owner'), ($2, 'registry-outsider')")
        .bind(owner_id)
        .bind(outsider_id)
        .execute(pool)
        .await
        .expect("seed registry readers");
    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, $2)")
        .bind(organization_id)
        .bind(format!("registry-org-{}", organization_id.simple()))
        .execute(pool)
        .await
        .expect("seed registry organization");
    sqlx::query(
        "INSERT INTO organization_members (organization_id, user_id, role) VALUES ($1, $2, 'owner')",
    )
    .bind(organization_id)
    .bind(owner_id)
    .execute(pool)
    .await
    .expect("seed registry owner membership");
    sqlx::query("INSERT INTO projects (id, organization_id, name) VALUES ($1, $2, $3)")
        .bind(project_id)
        .bind(organization_id)
        .bind(format!("registry-project-{}", project_id.simple()))
        .execute(pool)
        .await
        .expect("seed registry project");
    project_id
}

async fn assert_project_registry_rls(
    pool: &PgPool,
    owner_id: uuid::Uuid,
    outsider_id: uuid::Uuid,
    publication_id: uuid::Uuid,
) {
    assert_publication_visible_to(pool, owner_id, publication_id, true).await;
    assert_publication_visible_to(pool, outsider_id, publication_id, false).await;
}

async fn assert_publication_visible_to(
    pool: &PgPool,
    actor_id: uuid::Uuid,
    publication_id: uuid::Uuid,
    expected_visible: bool,
) {
    let mut transaction = pool.begin().await.expect("actor transaction");
    sqlx::query(
        "SELECT set_config('hephaestus.actor_id', $1, true),
                set_config('hephaestus.subject_type', 'user', true)",
    )
    .bind(actor_id.to_string())
    .execute(&mut *transaction)
    .await
    .expect("set actor");
    sqlx::query("SET LOCAL ROLE hephaestus_app")
        .execute(&mut *transaction)
        .await
        .expect("use application role");
    let visible =
        sqlx::query_scalar::<_, uuid::Uuid>("SELECT id FROM registry_publications WHERE id = $1")
            .bind(publication_id)
            .fetch_optional(&mut *transaction)
            .await
            .expect("project publication read");
    assert_eq!(visible.is_some(), expected_visible);
    transaction
        .rollback()
        .await
        .expect("close actor transaction");
}

async fn fixture() -> Option<PgPool> {
    let database_url = env::var("HEPHAESTUS_POSTGRES_TEST_URL").ok()?;
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .connect(&database_url)
        .await
        .expect("connect registry test database");
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("apply registry migrations");
    assert!(
        sqlx::query_scalar::<_, Option<String>>(
            "SELECT to_regclass('public.registry_publications')::text"
        )
        .fetch_one(&pool)
        .await
        .expect("inspect registry table")
        .is_some()
    );
    Some(pool)
}

fn intent() -> PublicationIntent {
    let owner = RegistryOwner::PlatformImage {
        image_key: PlatformImageKey::parse(format!("test-{}", uuid::Uuid::new_v4().simple()))
            .expect("platform key"),
    };
    let claim = NamespaceClaim::new(owner);
    let digest = digest('a');
    let reference = ImmutableManifestReference::new(
        RegistryAuthority::parse("registry.example").expect("authority"),
        claim.namespace().clone(),
        digest.clone(),
    );
    PublicationIntent::new(
        PublicationIntentId::new(),
        claim,
        reference,
        descriptor(digest, 100, OciMediaType::IMAGE_INDEX),
        PolicyVersion::parse("v1").expect("policy"),
        SupplyChainPolicy::without_signature(),
    )
    .expect("intent")
}

fn project_intent(project_id: uuid::Uuid, image_id: OciImageId) -> PublicationIntent {
    let owner = RegistryOwner::RepositoryOciImage {
        project_id: forge_domain::ProjectId::from_uuid(project_id),
        image_id,
    };
    let claim = NamespaceClaim::new(owner);
    let digest = digest('f');
    let reference = ImmutableManifestReference::new(
        RegistryAuthority::parse("registry.example").expect("authority"),
        claim.namespace().clone(),
        digest.clone(),
    );
    PublicationIntent::new(
        PublicationIntentId::new(),
        claim,
        reference,
        descriptor(digest, 100, OciMediaType::IMAGE_INDEX),
        PolicyVersion::parse("v1").expect("policy"),
        SupplyChainPolicy::without_signature(),
    )
    .expect("project intent")
}

fn verification(intent: &PublicationIntent) -> VerifiedPublication {
    let subject = intent.reference().digest().clone();
    let platform = PlatformDescriptor::new(
        descriptor(digest('b'), 99, OciMediaType::IMAGE_MANIFEST),
        "linux",
        "amd64",
        None,
    )
    .expect("platform");
    let evidence = SupplyChainEvidence::new(
        subject.clone(),
        vec![
            referrer(SupplyChainReferrerKind::Sbom, subject.clone(), 'c'),
            referrer(SupplyChainReferrerKind::Provenance, subject.clone(), 'd'),
            referrer(SupplyChainReferrerKind::Scan, subject, 'e'),
        ],
    )
    .expect("evidence");
    VerifiedPublication::new(
        intent.reference(),
        intent.expected_manifest().clone(),
        vec![platform],
        evidence,
    )
    .expect("verification")
}

fn referrer(
    kind: SupplyChainReferrerKind,
    subject: Sha256Digest,
    byte: char,
) -> SupplyChainReferrer {
    SupplyChainReferrer::new(
        kind,
        subject,
        descriptor(digest(byte), 42, "application/vnd.in-toto+json"),
        OciMediaType::parse("application/vnd.in-toto+json").expect("artifact type"),
    )
}

fn descriptor(digest: Sha256Digest, size: u64, media_type: &str) -> OciDescriptor {
    OciDescriptor::new(
        digest,
        size,
        OciMediaType::parse(media_type).expect("media type"),
    )
    .expect("descriptor")
}

fn digest(byte: char) -> Sha256Digest {
    Sha256Digest::parse(format!("sha256:{}", byte.to_string().repeat(64))).expect("digest")
}
