use super::model::{self, Delivery};
use crate::{
    application::event::{EventApplication, EventError, EventScope, ReadResult},
    event_cursor::EventCursorCodec,
    rpc::{RpcError, into_connect_error},
};
use buffa::Message as _;
use futures_util::StreamExt as _;
use identity_domain::AuthenticatedIdentity;
use tokio::sync::mpsc;

const DEFAULT_MAX_EVENTS: u32 = 256;
const MAX_EVENTS: u32 = 1_000;
const DEFAULT_MAX_TOTAL_BYTES: u64 = 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 16 * 1024 * 1024;
// Authorization is re-evaluated for every product-event delivery.
const READ_BATCH: i64 = 1;

pub(super) struct Frame {
    pub sequence: u64,
    pub committed_cursor: String,
    pub delivery: Delivery,
}

pub(super) async fn start(
    application: EventApplication,
    identity: AuthenticatedIdentity,
    scope: EventScope,
    resume_cursor: Option<&str>,
    max_events: u32,
    max_total_bytes: u64,
    codec: EventCursorCodec,
) -> Result<mpsc::Receiver<Result<Frame, connectrpc::ConnectError>>, connectrpc::ConnectError> {
    let max_events = if max_events == 0 {
        DEFAULT_MAX_EVENTS
    } else {
        max_events
    };
    let max_total_bytes = if max_total_bytes == 0 {
        DEFAULT_MAX_TOTAL_BYTES
    } else {
        max_total_bytes
    };
    if max_events > MAX_EVENTS || max_total_bytes > MAX_TOTAL_BYTES {
        return Err(into_connect_error(RpcError::InvalidArgument));
    }
    let resume = resume_cursor
        .filter(|value| !value.is_empty())
        .map(|value| parse_cursor(&codec, scope, value))
        .transpose()
        .map_err(into_connect_error)?;
    // Subscribe first so a commit/publication racing the snapshot is buffered.
    // Notifications are only wakeups; all delivery data still comes from the
    // durable cursor-ordered journal.
    let notifications = application.subscribe().await.map_err(map_error)?;
    let snapshot = application
        .snapshot(&identity, scope)
        .await
        .map_err(map_error)?;
    let initial = if let Some(resume) = resume {
        if resume.saturating_add(1) < snapshot.retained_from_cursor {
            Some((
                snapshot.committed_cursor,
                Delivery::Gap(
                    model::gap(
                        &codec,
                        scope,
                        resume,
                        snapshot.retained_from_cursor,
                        snapshot.committed_cursor,
                    )
                    .map_err(into_connect_error)?,
                ),
            ))
        } else {
            None
        }
    } else {
        let committed = snapshot.committed_cursor;
        Some((
            committed,
            Delivery::Barrier(
                model::barrier(&codec, scope, &snapshot).map_err(into_connect_error)?,
            ),
        ))
    };
    let cursor = resume.unwrap_or_else(|| snapshot_cursor(initial.as_ref()));
    let (sender, receiver) = mpsc::channel(16);
    tokio::spawn(async move {
        run(
            application,
            identity,
            scope,
            cursor,
            initial,
            max_events,
            max_total_bytes,
            notifications,
            codec,
            sender,
        )
        .await;
    });
    Ok(receiver)
}

fn snapshot_cursor(initial: Option<&(i64, Delivery)>) -> i64 {
    initial.map_or(0, |(cursor, _)| *cursor)
}

// The watch loop keeps its delivery, authorization, and budget state explicit.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
async fn run(
    application: EventApplication,
    identity: AuthenticatedIdentity,
    scope: EventScope,
    mut cursor: i64,
    initial: Option<(i64, Delivery)>,
    max_events: u32,
    max_total_bytes: u64,
    mut notifications: crate::application::event::EventWakeupStream,
    codec: EventCursorCodec,
    sender: mpsc::Sender<Result<Frame, connectrpc::ConnectError>>,
) {
    let mut sequence = 0_u64;
    let mut delivered = 0_u32;
    let mut delivered_bytes = 0_u64;
    if let Some((committed_cursor, delivery)) = initial {
        sequence += 1;
        let terminal = matches!(delivery, Delivery::Gap(_));
        let frame = Frame {
            sequence,
            committed_cursor: codec.encode(scope.kind.as_str(), scope.id, committed_cursor),
            delivery,
        };
        let bytes = encoded_frame_size(&frame);
        if bytes > max_total_bytes {
            let _ignored = sender
                .send(Err(into_connect_error(RpcError::ResourceExhausted)))
                .await;
            return;
        }
        delivered_bytes = bytes;
        if sender.send(Ok(frame)).await.is_err() || terminal {
            return;
        }
    }
    loop {
        if delivered >= max_events || delivered_bytes >= max_total_bytes {
            return;
        }
        let result = match application
            .read_after(&identity, scope, cursor, READ_BATCH)
            .await
        {
            Ok(result) => result,
            Err(EventError::PermissionDenied) => {
                sequence += 1;
                let frame = Frame {
                    sequence,
                    committed_cursor: codec.encode(scope.kind.as_str(), scope.id, cursor),
                    delivery: Delivery::Revoked(model::revoked(scope)),
                };
                let result = if delivered_bytes.saturating_add(encoded_frame_size(&frame))
                    > max_total_bytes
                {
                    Err(into_connect_error(RpcError::ResourceExhausted))
                } else {
                    Ok(frame)
                };
                let _ignored = sender.send(result).await;
                return;
            }
            Err(error) => {
                let _ignored = sender.send(Err(map_error(error))).await;
                return;
            }
        };
        match result {
            ReadResult::RetentionGap {
                requested_cursor,
                earliest_available_cursor,
                latest_committed_cursor,
            } => {
                let delivery = model::gap(
                    &codec,
                    scope,
                    requested_cursor,
                    earliest_available_cursor,
                    latest_committed_cursor,
                )
                .map(Delivery::Gap)
                .map_err(into_connect_error);
                sequence += 1;
                let frame = delivery.map(|delivery| Frame {
                    sequence,
                    committed_cursor: codec.encode(
                        scope.kind.as_str(),
                        scope.id,
                        latest_committed_cursor,
                    ),
                    delivery,
                });
                let result = frame.and_then(|frame| {
                    if delivered_bytes.saturating_add(encoded_frame_size(&frame)) > max_total_bytes
                    {
                        Err(into_connect_error(RpcError::ResourceExhausted))
                    } else {
                        Ok(frame)
                    }
                });
                let _ignored = sender.send(result).await;
                return;
            }
            ReadResult::Events {
                committed_cursor,
                values,
            } if values.is_empty() => {
                if committed_cursor < cursor {
                    let _ignored = sender
                        .send(Err(into_connect_error(RpcError::Internal)))
                        .await;
                    return;
                }
                if notifications.next().await.is_none() {
                    let _ignored = sender
                        .send(Err(into_connect_error(RpcError::Unavailable)))
                        .await;
                    return;
                }
            }
            ReadResult::Events {
                committed_cursor,
                values,
            } => {
                for value in values {
                    if value.cursor != cursor.saturating_add(1) {
                        let _ignored = sender
                            .send(Err(into_connect_error(RpcError::Internal)))
                            .await;
                        return;
                    }
                    cursor = value.cursor;
                    sequence += 1;
                    let delivery = match model::event(&codec, scope, &value) {
                        Ok(value) => Delivery::Event(value),
                        Err(error) => {
                            let _ignored = sender.send(Err(into_connect_error(error))).await;
                            return;
                        }
                    };
                    let frame = Frame {
                        sequence,
                        committed_cursor: codec.encode(
                            scope.kind.as_str(),
                            scope.id,
                            committed_cursor,
                        ),
                        delivery,
                    };
                    let event_bytes = encoded_frame_size(&frame);
                    if delivered_bytes.saturating_add(event_bytes) > max_total_bytes {
                        let _ignored = sender
                            .send(Err(into_connect_error(RpcError::ResourceExhausted)))
                            .await;
                        return;
                    }
                    delivered += 1;
                    delivered_bytes += event_bytes;
                    if sender.send(Ok(frame)).await.is_err() || delivered >= max_events {
                        return;
                    }
                }
            }
        }
    }
}

fn encoded_frame_size(frame: &Frame) -> u64 {
    let sequence = 1 + varint_len(frame.sequence);
    let cursor_text = u64::try_from(frame.committed_cursor.len()).unwrap_or(u64::MAX);
    let cursor_message = 1 + varint_len(cursor_text) + cursor_text;
    let cursor = 1 + varint_len(cursor_message) + cursor_message;
    let item_message = u64::from(match &frame.delivery {
        Delivery::Barrier(value) => value.encoded_len(),
        Delivery::Event(value) => value.encoded_len(),
        Delivery::Gap(value) => value.encoded_len(),
        Delivery::Revoked(value) => value.encoded_len(),
    });
    let item = 1 + varint_len(item_message) + item_message;
    // Connect streaming uses a five-byte envelope before every protobuf frame.
    5_u64
        .saturating_add(sequence)
        .saturating_add(cursor)
        .saturating_add(item)
}

const fn varint_len(mut value: u64) -> u64 {
    let mut len = 1;
    while value >= 0x80 {
        value >>= 7;
        len += 1;
    }
    len
}

fn parse_cursor(codec: &EventCursorCodec, scope: EventScope, value: &str) -> Result<i64, RpcError> {
    codec
        .decode(value, scope.kind.as_str(), scope.id)
        .ok_or(RpcError::InvalidArgument)
}

fn map_error(error: EventError) -> connectrpc::ConnectError {
    match error {
        EventError::PermissionDenied => into_connect_error(RpcError::PermissionDenied),
        EventError::InvalidCursor => into_connect_error(RpcError::InvalidArgument),
        EventError::Persistence(source) => {
            tracing::error!(error = %source, "event application persistence failed");
            into_connect_error(RpcError::Unavailable)
        }
        EventError::Notification(source) => {
            tracing::error!(error = %source, "event notification subscription failed");
            into_connect_error(RpcError::Unavailable)
        }
        EventError::ResourceExhausted => into_connect_error(RpcError::ResourceExhausted),
    }
}

#[cfg(test)]
mod tests {
    use super::{Delivery, Frame, encoded_frame_size, parse_cursor};
    use crate::{
        application::event::{EventScope, ScopeKind},
        event_cursor::EventCursorCodec,
    };
    use buffa::Message as _;
    use rpc_proto::messages::hephaestus::{
        common::v1::Cursor,
        event::v1::{ScopeSnapshotBarrier, WatchIdentityResponse, watch_identity_response},
    };
    use uuid::Uuid;

    #[test]
    fn resume_cursor_is_canonical_and_non_negative() {
        let codec = EventCursorCodec::new([4; 32]);
        let scope = EventScope {
            kind: ScopeKind::Repository,
            id: Uuid::new_v4(),
        };
        let zero = codec.encode(scope.kind.as_str(), scope.id, 0);
        let forty_two = codec.encode(scope.kind.as_str(), scope.id, 42);
        assert_eq!(parse_cursor(&codec, scope, &zero).expect("zero cursor"), 0);
        assert_eq!(parse_cursor(&codec, scope, &forty_two).expect("cursor"), 42);
        let other = EventScope {
            kind: ScopeKind::Project,
            id: scope.id,
        };
        assert!(parse_cursor(&codec, other, &forty_two).is_err());
        assert!(parse_cursor(&codec, scope, "not-a-cursor").is_err());
    }

    #[test]
    fn byte_budget_uses_exact_protobuf_and_connect_framing_size() {
        let barrier = ScopeSnapshotBarrier::default();
        let frame = Frame {
            sequence: 1,
            committed_cursor: String::from("signed-cursor"),
            delivery: Delivery::Barrier(barrier.clone()),
        };
        let response = WatchIdentityResponse {
            sequence: frame.sequence,
            committed_cursor: Cursor {
                value: frame.committed_cursor.clone(),
                ..Default::default()
            }
            .into(),
            item: Some(watch_identity_response::Item::SnapshotBarrier(Box::new(
                barrier,
            ))),
            ..Default::default()
        };
        assert_eq!(
            encoded_frame_size(&frame),
            u64::from(response.encoded_len()) + 5
        );
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn durable_watch_resumes_across_disconnect_gap_duplicate_wake_and_revocation() {
        use crate::{
            application::event::{EventApplication, EventWakeupSource},
            event_adapter::{EventPublisher, NatsEventWakeups, ensure_topology},
        };
        use async_nats::jetstream;
        use event_postgres::PostgresProductEventOutbox;
        use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
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
        let nats = async_nats::connect(nats_url)
            .await
            .expect("connect watch NATS");
        let jetstream = jetstream::new(nats.clone());
        ensure_topology(&jetstream)
            .await
            .expect("product event topology");
        let cursor_codec = EventCursorCodec::new([11; 32]);
        let publisher = EventPublisher::new(
            jetstream,
            Arc::new(PostgresProductEventOutbox::new(pool.clone())),
            [11; 32],
        );
        let wakeups: Arc<dyn EventWakeupSource> = Arc::new(NatsEventWakeups::new(nats.clone()));
        let application = EventApplication::new(pool.clone(), wakeups);

        let user_id = Uuid::new_v4();
        let organization_id = Uuid::new_v4();
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
        let scope = EventScope {
            kind: ScopeKind::Organization,
            id: organization_id,
        };

        let mut too_small = super::start(
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

        let mut receiver = super::start(
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

        mutate_organization(&pool, user_id, organization_id, "one").await;
        mutate_organization(&pool, user_id, organization_id, "two").await;
        publisher
            .publish_pending(100)
            .await
            .expect("publish disconnected changes");

        let restarted_application =
            EventApplication::new(pool.clone(), Arc::new(NatsEventWakeups::new(nats.clone())));
        let mut resumed = super::start(
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

        let mut duplicate_safe = super::start(
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
        mutate_organization(&pool, user_id, organization_id, "three").await;
        publisher
            .publish_pending(100)
            .await
            .expect("publish post-duplicate event");
        let unique = duplicate_safe
            .recv()
            .await
            .expect("unique event")
            .expect("valid unique event");
        assert!(matches!(unique.delivery, Delivery::Event(_)));
        drop(duplicate_safe);

        assert_connect_transport_resume(
            &pool,
            &nats,
            &publisher,
            user_id,
            organization_id,
            [11; 32],
        )
        .await;

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
        let mut gapped = super::start(
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
        assert!(matches!(gap.delivery, Delivery::Gap(_)));

        let mut post_prune = super::start(
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

        let mut revoked = super::start(
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
        publisher
            .publish_pending(100)
            .await
            .expect("publish revocation");
        let terminal = revoked
            .recv()
            .await
            .expect("revocation terminal")
            .expect("valid revocation terminal");
        assert!(matches!(terminal.delivery, Delivery::Revoked(_)));
    }

    async fn mutate_organization(
        pool: &sqlx::PgPool,
        actor: Uuid,
        organization: Uuid,
        suffix: &str,
    ) -> Uuid {
        let mut transaction = pool.begin().await.expect("organization transaction");
        let request = Uuid::new_v4();
        sqlx::query(
            "SELECT set_config('hephaestus.actor_id', $1, true),
                      set_config('hephaestus.subject_type', 'user', true),
                      set_config('hephaestus.request_id', $2, true),
                      set_config('hephaestus.occurrence_id', $2, true)",
        )
        .bind(actor.to_string())
        .bind(request.to_string())
        .execute(&mut *transaction)
        .await
        .expect("organization actor");
        sqlx::query("UPDATE organizations SET name = $2 WHERE id = $1")
            .bind(organization)
            .bind(format!("watch-{suffix}-{organization}"))
            .execute(&mut *transaction)
            .await
            .expect("mutate organization");
        transaction.commit().await.expect("commit organization");
        request
    }

    #[allow(clippy::too_many_lines)]
    async fn assert_connect_transport_resume(
        pool: &sqlx::PgPool,
        nats: &async_nats::Client,
        publisher: &crate::event_adapter::EventPublisher,
        user_id: Uuid,
        organization_id: Uuid,
        signing_key: [u8; 32],
    ) {
        use crate::{
            event_adapter::NatsEventWakeups,
            rpc::{MediatorAuthenticator, event::EventRpc},
        };
        use buffa::Message as _;
        use connectrpc::{
            Protocol, Router,
            client::{CallOptions, ClientConfig, HttpClient},
        };
        use futures_util::StreamExt as _;
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

        let service = Arc::new(EventRpc::new(
            pool.clone(),
            MediatorAuthenticator::new(&signing_key),
            Arc::new(NatsEventWakeups::new(nats.clone())),
            signing_key,
        ));
        let router = service.register(Router::new());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind Connect watch listener");
        let address = listener.local_addr().expect("Connect watch address");
        let server = tokio::spawn(async move {
            axum::serve(listener, router.into_axum_router())
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
        let token = mediator_assertion(&signing_key, user_id);
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
        let request_id = mutate_organization(pool, user_id, organization_id, "connect").await;
        let expected_event_id: Uuid = sqlx::query_scalar(
            "SELECT id FROM application_events
               WHERE scope_kind = 'organization' AND scope_id = $1
                 AND request_id = $2 AND aggregate_type = 'organization'",
        )
        .bind(organization_id)
        .bind(request_id)
        .fetch_one(pool)
        .await
        .expect("Connect event id");
        publisher
            .publish_pending(100)
            .await
            .expect("publish Connect event");

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

    fn mediator_assertion(signing_key: &[u8], user_id: Uuid) -> String {
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
        }

        let now = OffsetDateTime::now_utc().unix_timestamp();
        encode(
            &Header::new(Algorithm::HS256),
            &Claims {
                iss: "hephaestus-web-mediator",
                aud: "/hephaestus.event.v1.ProductEventService/WatchOrganization",
                sub: user_id.to_string(),
                jti: Uuid::new_v4().to_string(),
                iat: now,
                nbf: now,
                exp: now + 30,
            },
            &EncodingKey::from_secret(signing_key),
        )
        .expect("encode mediator assertion")
    }
}
