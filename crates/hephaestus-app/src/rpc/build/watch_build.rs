use super::{model, stream};
use crate::rpc::{into_connect_error, request};
use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult, ServiceStream};
use futures_util::stream as futures_stream;
use rpc_proto::messages::hephaestus::{
    build::v1::{WatchBuildRequest, WatchBuildResponse, watch_build_response::Item},
    common::v1::Cursor,
};
use tokio::sync::mpsc;
use tokio::time::{Duration, sleep};

const AUDIENCE: &str = "/hephaestus.build.v1.BuildService/WatchBuild";
const POLL_INTERVAL: Duration = Duration::from_secs(2);

// The generated streaming contract keeps authorization, cursor validation,
// and stream construction together for one resumable policy boundary.
#[allow(clippy::too_many_lines, clippy::unused_async)]
pub(super) async fn handle(
    service: &super::BuildRpc,
    ctx: RequestContext,
    request_message: ServiceRequest<'_, WatchBuildRequest>,
) -> ServiceResult<ServiceStream<WatchBuildResponse>> {
    let identity = request::query_identity(&ctx, &service.authenticator, AUDIENCE)
        .map_err(into_connect_error)?;
    let request = request_message.to_owned_message();
    let id = stream::parse_id(request.build_id.as_option()).map_err(into_connect_error)?;
    let (max_events, max_total_bytes) =
        stream::limits(request.max_events, request.max_total_bytes).map_err(into_connect_error)?;
    let resume_cursor = stream::validate_build_cursor(
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
        let mut cursor = resume_cursor;
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
            let current_cursor = crate::application::build::build_cursor(&build);
            let terminal = build.state.is_terminal();
            if cursor.as_deref() == Some(current_cursor.as_str()) {
                if terminal {
                    return;
                }
                sequence += 1;
                delivered += 1;
                let response = WatchBuildResponse {
                    sequence,
                    committed_cursor: Cursor {
                        value: current_cursor,
                        ..Default::default()
                    }
                    .into(),
                    item: Some(Item::Heartbeat(true)),
                    ..Default::default()
                };
                let bytes = 128_u64;
                if delivered_bytes.saturating_add(bytes) > max_total_bytes {
                    let _ignored = sender
                        .send(Err(into_connect_error(
                            crate::rpc::RpcError::ResourceExhausted,
                        )))
                        .await;
                    return;
                }
                delivered_bytes += bytes;
                if sender.send(Ok(response)).await.is_err() {
                    return;
                }
            } else {
                let bytes = build.logs.iter().map(|line| line.len() as u64).sum::<u64>() + 512;
                if delivered_bytes.saturating_add(bytes) > max_total_bytes {
                    let _ignored = sender
                        .send(Err(into_connect_error(
                            crate::rpc::RpcError::ResourceExhausted,
                        )))
                        .await;
                    return;
                }
                sequence += 1;
                delivered += 1;
                cursor = Some(current_cursor.clone());
                let response = WatchBuildResponse {
                    sequence,
                    committed_cursor: Cursor {
                        value: current_cursor,
                        ..Default::default()
                    }
                    .into(),
                    item: Some(Item::Build(Box::new(model::build(build)))),
                    ..Default::default()
                };
                delivered_bytes += bytes;
                if sender.send(Ok(response)).await.is_err() {
                    return;
                }
                if terminal {
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
