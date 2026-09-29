use super::super::model::{self, Delivery};
use super::{EventFilter, Frame, READ_BATCH, encoded_frame_size, map_error};
use crate::{
    application::event::{EventApplication, EventError, EventScope, ReadResult},
    event_cursor::EventCursorCodec,
    rpc::{
        RpcError, into_connect_error,
        request::{self, RequestBudget},
    },
};
use futures_util::StreamExt as _;
use identity_domain::AuthenticatedIdentity;
use std::future::Future;
use tokio::sync::mpsc;

pub(super) async fn run_with_stream_budget<T, Operation>(
    budget: &RequestBudget,
    sender: &mpsc::Sender<Result<Frame, connectrpc::ConnectError>>,
    operation: Operation,
) -> Result<Option<T>, RpcError>
where
    Operation: Future<Output = T>,
{
    tokio::select! {
        result = request::run_with_budget(budget, operation) => result.map(Some),
        () = sender.closed() => {
            budget.cancel();
            Ok(None)
        }
    }
}

async fn send_with_stream_budget(
    budget: &RequestBudget,
    sender: &mpsc::Sender<Result<Frame, connectrpc::ConnectError>>,
    item: Result<Frame, connectrpc::ConnectError>,
) -> bool {
    let sent = matches!(
        run_with_stream_budget(budget, sender, sender.send(item)).await,
        Ok(Some(Ok(())))
    );
    if !sent {
        budget.cancel();
    }
    sent
}

async fn send_stream_error(
    budget: &RequestBudget,
    sender: &mpsc::Sender<Result<Frame, connectrpc::ConnectError>>,
    error: RpcError,
) {
    if matches!(error, RpcError::DeadlineExceeded) {
        // The deadline has already won, so waiting through the normal budget
        // path cannot deliver the terminal status. The channel is bounded and
        // try_send keeps this best-effort notification nonblocking.
        let _ = sender.try_send(Err(into_connect_error(error)));
        budget.cancel();
    } else {
        let _ = send_with_stream_budget(budget, sender, Err(into_connect_error(error))).await;
    }
}

// The watch loop keeps its delivery, authorization, and budget state explicit.
// The loop intentionally keeps authorization, cursor, filtering, and budget
// transitions together so each delivery path shares identical semantics.
#[allow(
    clippy::cognitive_complexity,
    clippy::too_many_arguments,
    clippy::too_many_lines
)]
pub(super) async fn run(
    application: EventApplication,
    identity: AuthenticatedIdentity,
    scope: EventScope,
    mut cursor: i64,
    initial: Option<(i64, Delivery)>,
    max_events: u32,
    max_total_bytes: u64,
    mut notifications: crate::application::event::EventWakeupStream,
    codec: EventCursorCodec,
    filter: EventFilter,
    budget: RequestBudget,
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
            let _ = send_with_stream_budget(
                &budget,
                &sender,
                Err(into_connect_error(RpcError::ResourceExhausted)),
            )
            .await;
            return;
        }
        delivered_bytes = bytes;
        if !send_with_stream_budget(&budget, &sender, Ok(frame)).await || terminal {
            return;
        }
    }
    loop {
        if delivered >= max_events || delivered_bytes >= max_total_bytes {
            return;
        }
        let result = match run_with_stream_budget(
            &budget,
            &sender,
            application.read_after(&identity, scope, cursor, READ_BATCH),
        )
        .await
        {
            Ok(Some(Ok(result))) => result,
            Ok(Some(Err(EventError::PermissionDenied))) => {
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
                let _ = send_with_stream_budget(&budget, &sender, result).await;
                return;
            }
            Ok(Some(Err(error))) => {
                let _ = send_with_stream_budget(&budget, &sender, Err(map_error(error))).await;
                return;
            }
            Ok(None) => return,
            Err(error) => {
                send_stream_error(&budget, &sender, error).await;
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
                let _ = send_with_stream_budget(&budget, &sender, result).await;
                return;
            }
            ReadResult::Events {
                committed_cursor,
                values,
            } if values.is_empty() => {
                if committed_cursor < cursor {
                    let _ = send_with_stream_budget(
                        &budget,
                        &sender,
                        Err(into_connect_error(RpcError::Internal)),
                    )
                    .await;
                    return;
                }
                match run_with_stream_budget(&budget, &sender, notifications.next()).await {
                    Ok(Some(Some(()))) => {}
                    Ok(Some(None)) => {
                        let _ = send_with_stream_budget(
                            &budget,
                            &sender,
                            Err(into_connect_error(RpcError::Unavailable)),
                        )
                        .await;
                        return;
                    }
                    Ok(None) => return,
                    Err(error) => {
                        send_stream_error(&budget, &sender, error).await;
                        return;
                    }
                }
            }
            ReadResult::Events {
                committed_cursor,
                values,
            } => {
                for value in values {
                    if value.cursor != cursor.saturating_add(1) {
                        let _ = send_with_stream_budget(
                            &budget,
                            &sender,
                            Err(into_connect_error(RpcError::Internal)),
                        )
                        .await;
                        return;
                    }
                    cursor = value.cursor;
                    let delivery = match model::event(&codec, scope, &value) {
                        Ok(value) => Delivery::Event(value),
                        Err(error) => {
                            let _ = send_with_stream_budget(
                                &budget,
                                &sender,
                                Err(into_connect_error(error)),
                            )
                            .await;
                            return;
                        }
                    };
                    if let Delivery::Event(ref event) = delivery {
                        if !(filter)(event) {
                            continue;
                        }
                    }
                    sequence += 1;
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
                        let _ = send_with_stream_budget(
                            &budget,
                            &sender,
                            Err(into_connect_error(RpcError::ResourceExhausted)),
                        )
                        .await;
                        return;
                    }
                    delivered += 1;
                    delivered_bytes += event_bytes;
                    if !send_with_stream_budget(&budget, &sender, Ok(frame)).await
                        || delivered >= max_events
                    {
                        return;
                    }
                }
            }
        }
    }
}
