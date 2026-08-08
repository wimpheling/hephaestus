use super::{model, stream};
use crate::rpc::{RpcError, into_connect_error, request};
use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult, ServiceStream};
use futures_util::stream as futures_stream;
use rpc_proto::messages::hephaestus::{
    build::v1::{StreamBuildLogsRequest, StreamBuildLogsResponse},
    common::v1::Cursor,
};
use tokio::sync::mpsc;
use tokio::time::{Duration, sleep};

const AUDIENCE: &str = "/hephaestus.build.v1.BuildService/StreamBuildLogs";
const POLL_INTERVAL: Duration = Duration::from_secs(2);

// The generated streaming contract keeps authorization, bounds, and stream
// construction together so the persisted-log policy is auditable in one place.
#[allow(clippy::too_many_lines, clippy::unused_async)]
pub(super) async fn handle(
    service: &super::BuildRpc,
    ctx: RequestContext,
    request_message: ServiceRequest<'_, StreamBuildLogsRequest>,
) -> ServiceResult<ServiceStream<StreamBuildLogsResponse>> {
    let identity = request::query_identity(&ctx, &service.authenticator, AUDIENCE)
        .map_err(into_connect_error)?;
    let request = request_message.to_owned_message();
    let id = stream::parse_id(request.build_id.as_option()).map_err(into_connect_error)?;
    let (max_events, max_total_bytes) =
        stream::limits(request.max_events, request.max_total_bytes).map_err(into_connect_error)?;
    let mut offset = stream::parse_log_cursor(
        request
            .resume_cursor
            .as_option()
            .map(|cursor| cursor.value.as_str()),
        id,
    )
    .map_err(into_connect_error)?;
    let application = service.application.clone();
    let (sender, receiver) = mpsc::channel(8);
    tokio::spawn(async move {
        let mut sequence = 0_u64;
        let mut delivered = 0_u32;
        let mut delivered_bytes = 0_u64;
        loop {
            if delivered >= max_events {
                return;
            }
            let build = match application.get_build(&identity, id).await {
                Ok(build) => build,
                Err(error) => {
                    let _ignored = sender
                        .send(Err(into_connect_error(model::application_error(error))))
                        .await;
                    return;
                }
            };
            if offset > build.logs.len() {
                let _ignored = sender
                    .send(Err(into_connect_error(RpcError::FailedPrecondition)))
                    .await;
                return;
            }
            let mut emitted = false;
            while offset < build.logs.len() && delivered < max_events {
                let line = build.logs[offset].clone();
                let estimate = line.len() as u64 + 128;
                if delivered_bytes.saturating_add(estimate) > max_total_bytes {
                    let _ignored = sender
                        .send(Err(into_connect_error(RpcError::ResourceExhausted)))
                        .await;
                    return;
                }
                offset += 1;
                sequence += 1;
                delivered += 1;
                delivered_bytes += estimate;
                emitted = true;
                let terminal = build.state.is_terminal() && offset == build.logs.len();
                let response = StreamBuildLogsResponse {
                    sequence,
                    committed_cursor: Cursor {
                        value: stream::log_cursor(id, offset),
                        ..Default::default()
                    }
                    .into(),
                    contents: line,
                    end_of_stream: terminal,
                    truncated: !terminal && (delivered >= max_events),
                    ..Default::default()
                };
                if sender.send(Ok(response)).await.is_err() {
                    return;
                }
                if terminal {
                    return;
                }
            }
            if build.state.is_terminal() && offset == build.logs.len() {
                if !emitted {
                    sequence += 1;
                    let response = StreamBuildLogsResponse {
                        sequence,
                        committed_cursor: Cursor {
                            value: stream::log_cursor(id, offset),
                            ..Default::default()
                        }
                        .into(),
                        end_of_stream: true,
                        ..Default::default()
                    };
                    if sender.send(Ok(response)).await.is_err() {
                        return;
                    }
                }
                return;
            }
            if !emitted {
                sequence += 1;
                delivered += 1;
                if delivered_bytes.saturating_add(128) > max_total_bytes {
                    let _ignored = sender
                        .send(Err(into_connect_error(RpcError::ResourceExhausted)))
                        .await;
                    return;
                }
                let response = StreamBuildLogsResponse {
                    sequence,
                    committed_cursor: Cursor {
                        value: stream::log_cursor(id, offset),
                        ..Default::default()
                    }
                    .into(),
                    heartbeat: true,
                    ..Default::default()
                };
                delivered_bytes += 128;
                if sender.send(Ok(response)).await.is_err() {
                    return;
                }
            }
            sleep(POLL_INTERVAL).await;
        }
    });
    let responses = futures_stream::unfold(receiver, |mut receiver| async move {
        let response = receiver.recv().await?;
        Some((response, receiver))
    });
    Response::ok(Box::pin(responses))
}
