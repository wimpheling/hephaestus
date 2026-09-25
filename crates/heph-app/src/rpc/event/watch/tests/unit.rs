use crate::rpc::event::watch::producer::{run, run_with_stream_budget};
use crate::rpc::event::watch::{Delivery, Frame, encoded_frame_size, parse_cursor};
use crate::{
    application::event::{
        EventApplication, EventScope, EventWakeupSource, EventWakeupStream, ScopeKind,
    },
    event_cursor::EventCursorCodec,
    rpc::request::RequestBudget,
};
use buffa::Message as _;
use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
use rpc_proto::messages::hephaestus::{
    common::v1::Cursor,
    event::v1::{ScopeSnapshotBarrier, WatchIdentityResponse, watch_identity_response},
};
use sqlx::postgres::PgPoolOptions;
use tokio::sync::mpsc;
use uuid::Uuid;

struct NeverWakeups;

#[async_trait::async_trait]
impl EventWakeupSource for NeverWakeups {
    async fn subscribe(&self) -> Result<EventWakeupStream, String> {
        Ok(Box::pin(futures_util::stream::pending()))
    }
}

#[tokio::test]
async fn detached_watch_cancels_when_the_response_receiver_drops() {
    let budget = RequestBudget::unbounded();
    let cancellation = budget.cancellation_token();
    let (sender, receiver) = mpsc::channel(1);
    drop(receiver);

    let result = run_with_stream_budget(&budget, &sender, std::future::pending::<()>()).await;

    assert_eq!(result, Ok(None));
    assert!(cancellation.is_cancelled());
}

#[tokio::test]
async fn detached_watch_returns_deadline_error_without_resetting_the_deadline() {
    use std::time::{Duration, Instant};

    let deadline = Instant::now() + Duration::from_millis(1);
    let budget = RequestBudget::from_deadline(Some(deadline));
    let (sender, _receiver) = mpsc::channel(1);

    let result = run_with_stream_budget(&budget, &sender, std::future::pending::<()>()).await;

    assert_eq!(result, Err(crate::rpc::RpcError::DeadlineExceeded));
    assert_eq!(budget.deadline(), Some(deadline));
}

#[tokio::test]
async fn detached_watch_run_queues_deadline_error_before_stopping() {
    use std::time::{Duration, Instant};

    let application = EventApplication::new(
        PgPoolOptions::new()
            .connect_lazy("postgres://test:test@127.0.0.1:1/test")
            .expect("lazy test pool"),
        std::sync::Arc::new(NeverWakeups),
    );
    let identity = AuthenticatedIdentity::new(
        UserId::new(),
        "watch-test",
        "subject",
        serde_json::json!({}),
        RequestId::new(),
    );
    let scope = EventScope {
        kind: ScopeKind::Organization,
        id: Uuid::new_v4(),
    };
    let budget = RequestBudget::from_deadline(Some(
        Instant::now()
            .checked_sub(Duration::from_millis(1))
            .expect("deadline before now"),
    ));
    let cancellation = budget.cancellation_token();
    let (sender, mut receiver) = mpsc::channel(1);

    run(
        application,
        identity,
        scope,
        0,
        None,
        1,
        1024,
        Box::pin(futures_util::stream::pending()),
        EventCursorCodec::new([3; 32]),
        std::sync::Arc::new(|_| true),
        budget,
        sender,
    )
    .await;

    let response = receiver.recv().await.expect("deadline terminal response");
    let Err(error) = response else {
        panic!("deadline must be returned as a stream error");
    };
    assert_eq!(error.code, connectrpc::ErrorCode::DeadlineExceeded);
    assert!(cancellation.is_cancelled());
}

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
