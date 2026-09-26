use async_nats::Client as NatsClient;
use connectrpc::{
    Protocol, Router,
    client::{CallOptions, ClientConfig, HttpClient},
};
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
use sqlx::PgPool;
use std::{sync::Arc, time::Duration};
use tokio::task::JoinHandle;

type ConnectClient = ProductEventServiceClient<HttpClient>;

pub(super) struct ConnectWatchContext<'a> {
    pub(super) pool: &'a PgPool,
    pub(super) nats: &'a NatsClient,
    pub(super) publisher: &'a crate::event_adapter::EventPublisher,
    pub(super) user_id: super::Uuid,
    pub(super) organization_id: super::Uuid,
    pub(super) application_pool: &'a PgPool,
    pub(super) worker_pool: &'a PgPool,
    pub(super) signing_key: [u8; 32],
    pub(super) sid: identity_domain::BrowserSessionSid,
}

struct ConnectWatchRuntime {
    client: ConnectClient,
    server: JoinHandle<()>,
    token: String,
}

pub(super) async fn assert_connect_transport_resume(context: ConnectWatchContext<'_>) {
    let runtime = start_runtime(&context).await;
    let mut initial = runtime
        .client
        .watch_organization_with_options(
            watch_request(&context, None),
            call_options(&runtime.token),
        )
        .await
        .expect("Connect watch starts");
    let barrier_cursor = receive_barrier(&mut initial).await;
    drop(initial);

    let expected_event_id = publish_event(&context).await;
    let mut resumed = runtime
        .client
        .watch_organization_with_options(
            watch_request(&context, Some(barrier_cursor)),
            call_options(&runtime.token),
        )
        .await
        .expect("Connect watch resumes");
    assert_resumed_event(&mut resumed, expected_event_id).await;
    assert_duplicate_wakes_do_not_duplicate(&context, &mut resumed).await;

    runtime.server.abort();
    let _result = runtime.server.await;
}

async fn start_runtime(context: &ConnectWatchContext<'_>) -> ConnectWatchRuntime {
    use crate::{
        event_adapter::NatsEventWakeups,
        rpc::{
            MediatorAuthenticationState, MediatorAuthenticator, event::EventRpc,
            mediator_identity_middleware,
        },
    };
    use axum::middleware::from_fn_with_state;
    use identity_postgres::PostgresBrowserSessionStore;

    let browser_sessions = Arc::new(PostgresBrowserSessionStore::new(
        context.worker_pool.clone(),
        context.application_pool.clone(),
    ));
    let service = Arc::new(EventRpc::new(
        context.pool.clone(),
        MediatorAuthenticator::new(&context.signing_key),
        Arc::new(NatsEventWakeups::new(context.nats.clone())),
        context.signing_key,
    ));
    let auth_state = MediatorAuthenticationState::new(
        MediatorAuthenticator::new(&context.signing_key),
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
    let token = mediator_assertion(&context.signing_key, context.user_id, context.sid);
    ConnectWatchRuntime {
        client,
        server,
        token,
    }
}

fn call_options(token: &str) -> CallOptions {
    CallOptions::default()
        .with_header("authorization", format!("Bearer {token}"))
        .with_timeout(Duration::from_secs(5))
}

fn watch_request(
    context: &ConnectWatchContext<'_>,
    resume: Option<String>,
) -> WatchOrganizationRequest {
    WatchOrganizationRequest {
        organization_id: OpaqueId {
            value: context.organization_id.to_string(),
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
    }
}

async fn receive_barrier(
    stream: &mut connectrpc::client::ServerStream<
        <HttpClient as connectrpc::client::ClientTransport>::ResponseBody,
        rpc_proto::messages::hephaestus::event::v1::__buffa::view::WatchOrganizationResponseView<
            'static,
        >,
    >,
) -> String {
    let barrier = stream
        .message::<WatchOrganizationResponse>()
        .await
        .expect("Connect barrier frame")
        .expect("Connect barrier present")
        .to_owned_message();
    assert!(matches!(
        barrier.item,
        Some(watch_organization_response::Item::SnapshotBarrier(_))
    ));
    barrier
        .committed_cursor
        .as_option()
        .expect("Connect barrier cursor")
        .value
        .clone()
}

async fn publish_event(context: &ConnectWatchContext<'_>) -> super::Uuid {
    use buffa::Message as _;
    use futures_util::StreamExt as _;

    let mut typed_messages = context
        .nats
        .subscribe(crate::event_adapter::PRODUCT_EVENT_SUBJECT)
        .await
        .expect("typed product-event subscription");
    let request_id = super::fixtures::mutate_organization(
        context.pool,
        context.user_id,
        context.organization_id,
        "connect",
    )
    .await;
    let expected_event_id: super::Uuid = sqlx::query_scalar(
        "SELECT id FROM application_events
           WHERE scope_kind = 'organization' AND scope_id = $1
             AND request_id = $2 AND aggregate_type = 'organization'",
    )
    .bind(context.organization_id)
    .bind(request_id)
    .fetch_one(context.pool)
    .await
    .expect("Connect event id");
    super::fixtures::publish_events_until_published(
        context.pool,
        context.publisher,
        &[expected_event_id],
    )
    .await;

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
    expected_event_id
}

async fn assert_resumed_event(
    stream: &mut connectrpc::client::ServerStream<
        <HttpClient as connectrpc::client::ClientTransport>::ResponseBody,
        rpc_proto::messages::hephaestus::event::v1::__buffa::view::WatchOrganizationResponseView<
            'static,
        >,
    >,
    expected_event_id: super::Uuid,
) {
    let event = stream
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
}

async fn assert_duplicate_wakes_do_not_duplicate(
    context: &ConnectWatchContext<'_>,
    stream: &mut connectrpc::client::ServerStream<
        <HttpClient as connectrpc::client::ClientTransport>::ResponseBody,
        rpc_proto::messages::hephaestus::event::v1::__buffa::view::WatchOrganizationResponseView<
            'static,
        >,
    >,
) {
    for _duplicate in 0..2 {
        context
            .nats
            .publish(
                crate::event_adapter::PRODUCT_EVENT_SUBJECT,
                Vec::new().into(),
            )
            .await
            .expect("duplicate Connect wake");
    }
    context
        .nats
        .flush()
        .await
        .expect("flush duplicate Connect wakes");
    assert!(
        tokio::time::timeout(
            Duration::from_millis(200),
            stream.message::<WatchOrganizationResponse>(),
        )
        .await
        .is_err(),
        "duplicate wakes must not duplicate a Connect event"
    );
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
