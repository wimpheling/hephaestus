pub(super) async fn assert_connect_transport_resume(
    pool: &sqlx::PgPool,
    nats: &async_nats::Client,
    publisher: &crate::event_adapter::EventPublisher,
    user_id: super::Uuid,
    organization_id: super::Uuid,
    application_pool: &sqlx::PgPool,
    worker_pool: &sqlx::PgPool,
    signing_key: [u8; 32],
    sid: identity_domain::BrowserSessionSid,
) {
    use crate::{
        event_adapter::NatsEventWakeups,
        rpc::{
            MediatorAuthenticationState, MediatorAuthenticator, event::EventRpc,
            mediator_identity_middleware,
        },
    };
    use axum::middleware::from_fn_with_state;
    use buffa::Message as _;
    use connectrpc::{
        Protocol, Router,
        client::{CallOptions, ClientConfig, HttpClient},
    };
    use futures_util::StreamExt as _;
    use identity_postgres::PostgresBrowserSessionStore;
    use rpc_proto::{
        connect::hephaestus::event::v1::{ProductEventServiceClient, ProductEventServiceExt},
        messages::hephaestus::{
            common::v1::{Cursor, OpaqueId},
            event::v1::{
                ProductEvent, WatchOrganizationRequest, WatchOrganizationResponse,
                watch_organization_response,
            },
        },
    };
    use std::{sync::Arc, time::Duration};

    let browser_sessions = Arc::new(PostgresBrowserSessionStore::new(
        worker_pool.clone(),
        application_pool.clone(),
    ));
    let service = Arc::new(EventRpc::new(
        pool.clone(),
        MediatorAuthenticator::new(&signing_key),
        Arc::new(NatsEventWakeups::new(nats.clone())),
        signing_key,
    ));
    let auth_state = MediatorAuthenticationState::new(
        MediatorAuthenticator::new(&signing_key),
        browser_sessions,
    );
    let router = service
        .register(Router::new())
        .into_axum_router()
        .layer(from_fn_with_state(auth_state, mediator_identity_middleware));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind Connect watch listener");
    let address = listener.local_addr().expect("Connect watch address");
    let server = tokio::spawn(async move {
        axum::serve(listener, router)
            .await
            .expect("serve Connect watch");
    });
    let uri = format!("http://{address}")
        .parse()
        .expect("Connect watch URI");
    let client = ProductEventServiceClient::new(
        HttpClient::plaintext(),
        ClientConfig::new(uri).with_protocol(Protocol::Connect),
    );
    let token = mediator_assertion(&signing_key, user_id, sid);
    let options = || {
        CallOptions::default()
            .with_header("authorization", format!("Bearer {token}"))
            .with_timeout(Duration::from_secs(5))
    };
    let request = |resume: Option<String>| WatchOrganizationRequest {
        organization_id: OpaqueId {
            value: organization_id.to_string(),
            ..Default::default()
        }
        .into(),
        resume_cursor: resume
            .map(|value| Cursor {
                value,
                ..Default::default()
            })
            .into(),
        max_events: 5,
        max_total_bytes: 1024 * 1024,
        ..Default::default()
    };

    let mut initial = client
        .watch_organization_with_options(request(None), options())
        .await
        .expect("Connect watch starts");
    let barrier = initial
        .message::<WatchOrganizationResponse>()
        .await
        .expect("Connect barrier frame")
        .expect("Connect barrier present")
        .to_owned_message();
    assert!(matches!(
        barrier.item,
        Some(watch_organization_response::Item::SnapshotBarrier(_))
    ));
    let barrier_cursor = barrier
        .committed_cursor
        .as_option()
        .expect("Connect barrier cursor")
        .value
        .clone();
    drop(initial);

    let mut typed_messages = nats
        .subscribe(crate::event_adapter::PRODUCT_EVENT_SUBJECT)
        .await
        .expect("typed product-event subscription");
    let request_id =
        super::fixtures::mutate_organization(pool, user_id, organization_id, "connect").await;
    let expected_event_id: super::Uuid = sqlx::query_scalar(
        "SELECT id FROM application_events
           WHERE scope_kind = 'organization' AND scope_id = $1
             AND request_id = $2 AND aggregate_type = 'organization'",
    )
    .bind(organization_id)
    .bind(request_id)
    .fetch_one(pool)
    .await
    .expect("Connect event id");
    super::fixtures::publish_events_until_published(pool, publisher, &[expected_event_id]).await;

    let decoded = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let message = typed_messages.next().await.expect("product-event message");
            let event = ProductEvent::decode_from_slice(message.payload.as_ref())
                .expect("typed ProductEvent protobuf");
            if event
                .event_id
                .as_option()
                .is_some_and(|id| id.value == expected_event_id.to_string())
            {
                break event;
            }
        }
    })
    .await
    .expect("typed ProductEvent arrives");
    assert_eq!(decoded.schema_version, 1);
    assert!(decoded.payload.is_some());

    let mut resumed = client
        .watch_organization_with_options(request(Some(barrier_cursor)), options())
        .await
        .expect("Connect watch resumes");
    let event = resumed
        .message::<WatchOrganizationResponse>()
        .await
        .expect("Connect event frame")
        .expect("Connect event present")
        .to_owned_message();
    let Some(watch_organization_response::Item::Event(event)) = event.item else {
        panic!("resumed Connect watch must deliver an event");
    };
    assert_eq!(
        event.event_id.as_option().map(|id| id.value.as_str()),
        Some(expected_event_id.to_string().as_str())
    );

    for _duplicate in 0..2 {
        nats.publish(
            crate::event_adapter::PRODUCT_EVENT_SUBJECT,
            Vec::new().into(),
        )
        .await
        .expect("duplicate Connect wake");
    }
    nats.flush().await.expect("flush duplicate Connect wakes");
    assert!(
        tokio::time::timeout(
            Duration::from_millis(200),
            resumed.message::<WatchOrganizationResponse>(),
        )
        .await
        .is_err(),
        "duplicate wakes must not duplicate a Connect event"
    );
    server.abort();
    let _result = server.await;
}

fn mediator_assertion(
    signing_key: &[u8],
    user_id: super::Uuid,
    sid: identity_domain::BrowserSessionSid,
) -> String {
    use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
    use serde::Serialize;
    use time::OffsetDateTime;

    #[derive(Serialize)]
    struct Claims<'a> {
        iss: &'a str,
        aud: &'a str,
        sub: String,
        jti: String,
        iat: i64,
        nbf: i64,
        exp: i64,
        sid: String,
    }

    let now = OffsetDateTime::now_utc().unix_timestamp();
    encode(
        &Header::new(Algorithm::HS256),
        &Claims {
            iss: "hephaestus-web-mediator",
            aud: "/hephaestus.event.v1.ProductEventService/WatchOrganization",
            sub: user_id.to_string(),
            jti: super::Uuid::new_v4().to_string(),
            iat: now,
            nbf: now,
            exp: now + 30,
            sid: sid.to_protocol_string(),
        },
        &EncodingKey::from_secret(signing_key),
    )
    .expect("encode mediator assertion")
}
