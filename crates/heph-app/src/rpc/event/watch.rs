use super::model::{self, Delivery};
use crate::{
    application::event::{EventApplication, EventError, EventScope},
    event_cursor::EventCursorCodec,
    rpc::{
        RpcError, into_connect_error,
        request::{self, RequestBudget},
    },
};
use buffa::Message as _;
use identity_domain::AuthenticatedIdentity;
use rpc_proto::messages::hephaestus::event::v1::ProductEvent;
use std::sync::Arc;
use tokio::sync::mpsc;

#[path = "watch/producer.rs"]
mod producer;
#[cfg(test)]
#[path = "watch/tests.rs"]
mod tests;

const DEFAULT_MAX_EVENTS: u32 = 256;
const MAX_EVENTS: u32 = 1_000;
const DEFAULT_MAX_TOTAL_BYTES: u64 = 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 16 * 1024 * 1024;
// Authorization is re-evaluated for every product-event delivery.
const READ_BATCH: i64 = 1;

pub(crate) struct Frame {
    pub sequence: u64,
    pub committed_cursor: String,
    pub delivery: Delivery,
}

#[cfg(test)]
pub(crate) async fn start(
    application: EventApplication,
    identity: AuthenticatedIdentity,
    scope: EventScope,
    resume_cursor: Option<&str>,
    max_events: u32,
    max_total_bytes: u64,
    codec: EventCursorCodec,
) -> Result<mpsc::Receiver<Result<Frame, connectrpc::ConnectError>>, connectrpc::ConnectError> {
    start_with_budget(
        application,
        identity,
        scope,
        resume_cursor,
        max_events,
        max_total_bytes,
        codec,
        RequestBudget::unbounded(),
    )
    .await
}

/// Starts a durable scope watch with an owned request budget.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn start_with_budget(
    application: EventApplication,
    identity: AuthenticatedIdentity,
    scope: EventScope,
    resume_cursor: Option<&str>,
    max_events: u32,
    max_total_bytes: u64,
    codec: EventCursorCodec,
    budget: RequestBudget,
) -> Result<mpsc::Receiver<Result<Frame, connectrpc::ConnectError>>, connectrpc::ConnectError> {
    start_filtered_with_budget(
        application,
        identity,
        scope,
        resume_cursor,
        max_events,
        max_total_bytes,
        codec,
        Arc::new(accept_all),
        budget,
    )
    .await
}

/// Starts a durable scope watch while retaining only the requested typed
/// product-event family. The cursor still advances over every event in the
/// authorized scope, so reconnects cannot replay unrelated events forever.
// The arguments mirror the existing public watch contract and keep the filter
// explicit at this policy boundary.
#[cfg(test)]
#[allow(dead_code)]
#[allow(clippy::too_many_arguments)]
pub(crate) async fn start_filtered(
    application: EventApplication,
    identity: AuthenticatedIdentity,
    scope: EventScope,
    resume_cursor: Option<&str>,
    max_events: u32,
    max_total_bytes: u64,
    codec: EventCursorCodec,
    filter: EventFilter,
) -> Result<mpsc::Receiver<Result<Frame, connectrpc::ConnectError>>, connectrpc::ConnectError> {
    start_filtered_with_budget(
        application,
        identity,
        scope,
        resume_cursor,
        max_events,
        max_total_bytes,
        codec,
        filter,
        RequestBudget::unbounded(),
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn start_filtered_with_budget(
    application: EventApplication,
    identity: AuthenticatedIdentity,
    scope: EventScope,
    resume_cursor: Option<&str>,
    max_events: u32,
    max_total_bytes: u64,
    codec: EventCursorCodec,
    filter: EventFilter,
    budget: RequestBudget,
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
    let notifications = request::run_with_budget(&budget, application.subscribe())
        .await
        .map_err(into_connect_error)?
        .map_err(map_error)?;
    let snapshot = request::run_with_budget(&budget, application.snapshot(&identity, scope))
        .await
        .map_err(into_connect_error)?
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
        producer::run(
            application,
            identity,
            scope,
            cursor,
            initial,
            max_events,
            max_total_bytes,
            notifications,
            codec,
            filter,
            budget,
            sender,
        )
        .await;
    });
    Ok(receiver)
}

fn snapshot_cursor(initial: Option<&(i64, Delivery)>) -> i64 {
    initial.map_or(0, |(cursor, _)| *cursor)
}

pub(crate) type EventFilter = Arc<dyn Fn(&ProductEvent) -> bool + Send + Sync>;

const fn accept_all(_: &ProductEvent) -> bool {
    true
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
