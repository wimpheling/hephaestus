//! Opt-in integration coverage for the registry control-plane migration.

#[path = "postgres/support.rs"]
mod support;
#[path = "postgres/values.rs"]
mod values;

use builder_catalog_domain::OciImageId;
use registry_postgres::{
    NewRegistryNotification, NotificationCompletion, PgRegistryStore, RegistryNotificationAction,
    RegistryNotificationTarget,
};
use serial_test::serial;
use support::{
    assert_platform_catalog_rls, assert_project_registry_rls, fixture, seed_project_reader,
};
use time::OffsetDateTime;
use values::{intent, project_intent, verification};

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
