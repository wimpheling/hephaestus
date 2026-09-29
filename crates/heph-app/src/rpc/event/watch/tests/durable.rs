use super::fixtures as f;

#[tokio::test]
#[serial_test::serial]
async fn durable_watch_resumes_across_disconnect_gap_duplicate_wake_and_revocation() {
    use crate::{
        application::event::{EventApplication, EventWakeupSource},
        event_adapter::{EventPublisher, NatsEventWakeups, ensure_topology},
    };
    use async_nats::jetstream;
    use event_postgres::PostgresProductEventOutbox;
    use identity_domain::{
        AuthenticatedIdentity, BrowserSessionSid, RequestId, UserId,
        browser_session_identity_binding_digest, browser_session_sid_digest,
    };
    use serde_json::json;
    use sqlx::postgres::PgPoolOptions;
    use std::{sync::Arc, time::Duration};

    let (Ok(database_url), Ok(nats_url)) = (
        std::env::var("HEPHAESTUS_POSTGRES_TEST_URL"),
        std::env::var("HEPHAESTUS_NATS_TEST_URL"),
    ) else {
        return;
    };
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .expect("connect watch PostgreSQL");
    let worker_pool = PgPoolOptions::new()
        .max_connections(2)
        .after_connect(|connection, _metadata| {
            Box::pin(async move {
                sqlx::query("SET ROLE hephaestus_worker")
                    .execute(connection)
                    .await
                    .map(|_| ())
            })
        })
        .connect(&database_url)
        .await
        .expect("connect watch worker PostgreSQL");
    let application_pool = PgPoolOptions::new()
        .max_connections(2)
        .after_connect(|connection, _metadata| {
            Box::pin(async move {
                sqlx::query("SET ROLE hephaestus_app")
                    .execute(connection)
                    .await
                    .map(|_| ())
            })
        })
        .connect(&database_url)
        .await
        .expect("connect watch application PostgreSQL");
    let nats = async_nats::connect(nats_url)
        .await
        .expect("connect watch NATS");
    let jetstream = jetstream::new(nats.clone());
    ensure_topology(&jetstream)
        .await
        .expect("product event topology");
    let cursor_codec = super::EventCursorCodec::new([11; 32]);
    let publisher = EventPublisher::new(
        jetstream,
        Arc::new(PostgresProductEventOutbox::new(pool.clone())),
        [11; 32],
    );
    let wakeups: Arc<dyn EventWakeupSource> = Arc::new(NatsEventWakeups::new(nats.clone()));
    let application = EventApplication::new(pool.clone(), wakeups);

    let user_id = super::Uuid::new_v4();
    let organization_id = super::Uuid::new_v4();
    sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, 'Watch Actor')")
        .bind(user_id)
        .execute(&pool)
        .await
        .expect("watch user");
    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, $2)")
        .bind(organization_id)
        .bind(format!("watch-{organization_id}"))
        .execute(&pool)
        .await
        .expect("watch organization");
    sqlx::query(
        "INSERT INTO organization_members (organization_id, user_id, role)
           VALUES ($1, $2, 'owner')",
    )
    .bind(organization_id)
    .bind(user_id)
    .execute(&pool)
    .await
    .expect("watch membership");
    let identity = AuthenticatedIdentity::new(
        UserId::from_uuid(user_id),
        "watch-test",
        user_id.to_string(),
        json!({}),
        RequestId::new(),
    );
    let sid = BrowserSessionSid::new();
    sqlx::query(
        "INSERT INTO human_browser_sessions
            (id, sid_digest, creation_idempotency_id, creation_request_id,
             identity_binding_digest, user_id, issued_at, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, now(), now() + interval '12 hours')",
    )
    .bind(super::Uuid::new_v4())
    .bind(browser_session_sid_digest(sid).as_bytes().to_vec())
    .bind(super::Uuid::new_v4())
    .bind(super::Uuid::new_v4())
    .bind(
        browser_session_identity_binding_digest(&identity)
            .as_bytes()
            .to_vec(),
    )
    .bind(user_id)
    .execute(&worker_pool)
    .await
    .expect("seed active watch browser session");
    let scope = super::EventScope {
        kind: super::ScopeKind::Organization,
        id: organization_id,
    };

    let mut too_small = crate::rpc::event::watch::start(
        application.clone(),
        identity.clone(),
        scope,
        None,
        1,
        1,
        cursor_codec.clone(),
    )
    .await
    .expect("bounded watch starts");
    assert!(
        too_small
            .recv()
            .await
            .expect("bounded watch terminal")
            .is_err()
    );

    let mut receiver = crate::rpc::event::watch::start(
        application.clone(),
        identity.clone(),
        scope,
        None,
        20,
        1024 * 1024,
        cursor_codec.clone(),
    )
    .await
    .expect("new watch");
    let barrier = receiver
        .recv()
        .await
        .expect("snapshot barrier")
        .expect("valid snapshot barrier");
    let barrier_cursor = barrier.committed_cursor;
    drop(receiver);

    let first_request = f::mutate_organization(&pool, user_id, organization_id, "one").await;
    let second_request = f::mutate_organization(&pool, user_id, organization_id, "two").await;
    let disconnected_events = vec![
        f::product_event_id(&pool, first_request).await,
        f::product_event_id(&pool, second_request).await,
    ];
    f::publish_events_until_published(&pool, &publisher, &disconnected_events).await;

    let restarted_application =
        EventApplication::new(pool.clone(), Arc::new(NatsEventWakeups::new(nats.clone())));
    let mut resumed = crate::rpc::event::watch::start(
        restarted_application.clone(),
        identity.clone(),
        scope,
        Some(&barrier_cursor),
        20,
        1024 * 1024,
        cursor_codec.clone(),
    )
    .await
    .expect("resumed watch");
    let first = resumed
        .recv()
        .await
        .expect("first resumed event")
        .expect("valid first event");
    let second = resumed
        .recv()
        .await
        .expect("second resumed event")
        .expect("valid second event");
    assert_eq!(second.sequence, first.sequence + 1);
    let resume_cursor = second.committed_cursor.clone();
    drop(resumed);

    let mut duplicate_safe = crate::rpc::event::watch::start(
        restarted_application.clone(),
        identity.clone(),
        scope,
        Some(&resume_cursor),
        20,
        1024 * 1024,
        cursor_codec.clone(),
    )
    .await
    .expect("duplicate-safe watch");
    nats.publish(
        crate::event_adapter::PRODUCT_EVENT_SUBJECT,
        Vec::new().into(),
    )
    .await
    .expect("first duplicate wake");
    nats.publish(
        crate::event_adapter::PRODUCT_EVENT_SUBJECT,
        Vec::new().into(),
    )
    .await
    .expect("second duplicate wake");
    assert!(
        tokio::time::timeout(Duration::from_millis(150), duplicate_safe.recv())
            .await
            .is_err(),
        "duplicate wakeups must not duplicate a durable event"
    );
    let third_request = f::mutate_organization(&pool, user_id, organization_id, "three").await;
    let third_event = f::product_event_id(&pool, third_request).await;
    f::publish_events_until_published(&pool, &publisher, &[third_event]).await;
    let unique = duplicate_safe
        .recv()
        .await
        .expect("unique event")
        .expect("valid unique event");
    assert!(matches!(unique.delivery, super::Delivery::Event(_)));
    drop(duplicate_safe);

    super::connect::assert_connect_transport_resume(super::connect::ConnectWatchContext {
        pool: &pool,
        nats: &nats,
        publisher: &publisher,
        user_id,
        organization_id,
        application_pool: &application_pool,
        worker_pool: &worker_pool,
        signing_key: [11; 32],
        sid,
    })
    .await;

    worker_pool.close().await;
    application_pool.close().await;

    sqlx::query(
        "UPDATE application_events SET retained_until = now() - interval '1 second'
           WHERE scope_kind = 'organization' AND scope_id = $1",
    )
    .bind(organization_id)
    .execute(&pool)
    .await
    .expect("expire organization events");
    sqlx::query("SELECT prune_application_events(10000)")
        .execute(&pool)
        .await
        .expect("prune organization events");
    let mut gapped = crate::rpc::event::watch::start(
        restarted_application.clone(),
        identity.clone(),
        scope,
        Some(&barrier_cursor),
        20,
        1024 * 1024,
        cursor_codec.clone(),
    )
    .await
    .expect("gapped watch");
    let gap = gapped
        .recv()
        .await
        .expect("retention gap")
        .expect("valid retention gap");
    assert!(matches!(gap.delivery, super::Delivery::Gap(_)));

    let mut post_prune = crate::rpc::event::watch::start(
        restarted_application.clone(),
        identity.clone(),
        scope,
        None,
        20,
        1024 * 1024,
        cursor_codec.clone(),
    )
    .await
    .expect("post-prune watch");
    let post_prune_barrier = post_prune
        .recv()
        .await
        .expect("post-prune barrier")
        .expect("valid post-prune barrier");
    let post_prune_cursor = post_prune_barrier.committed_cursor;
    drop(post_prune);

    let mut revoked = crate::rpc::event::watch::start(
        restarted_application,
        identity,
        scope,
        Some(&post_prune_cursor),
        20,
        1024 * 1024,
        cursor_codec,
    )
    .await
    .expect("revocation watch");
    sqlx::query(
        "DELETE FROM organization_members
           WHERE organization_id = $1 AND user_id = $2",
    )
    .bind(organization_id)
    .bind(user_id)
    .execute(&pool)
    .await
    .expect("revoke membership");
    let revocation_events = f::unpublished_scope_event_ids(&pool, organization_id).await;
    assert!(
        !revocation_events.is_empty(),
        "membership revocation event exists"
    );
    f::publish_events_until_published(&pool, &publisher, &revocation_events).await;
    // The membership deletion races a read that began while the watch was
    // still authorized. That read may deliver its already-committed event;
    // the next authorization check must then terminate with revocation.
    let terminal = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let frame = revoked
                .recv()
                .await
                .expect("revocation terminal")
                .expect("valid revocation terminal");
            if matches!(frame.delivery, super::Delivery::Revoked(_)) {
                break frame;
            }
            assert!(matches!(frame.delivery, super::Delivery::Event(_)));
        }
    })
    .await
    .expect("event watch must observe revocation");
    assert!(matches!(terminal.delivery, super::Delivery::Revoked(_)));
}
