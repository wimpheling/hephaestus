use super::{RepositoryBrowserRpc, map_error, parse_id};
use crate::rpc::{RpcError, into_connect_error, request};
use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult, ServiceStream};
use futures_util::stream;
use rpc_proto::messages::hephaestus::{
    common::v1::Cursor,
    repository_browser::v1::{StreamFileRequest, StreamFileResponse},
};
use std::time::Instant;
use tokio::{sync::mpsc, time::sleep_until};
use tokio_util::sync::CancellationToken;

const DEFAULT_TOTAL: usize = 16 * 1_048_576;
const MAX_TOTAL: usize = 16 * 1_048_576;
const DEFAULT_CHUNK: usize = 64 * 1_024;
const MAX_CHUNK: usize = 1_048_576;

pub(super) async fn handle(
    service: &RepositoryBrowserRpc,
    ctx: RequestContext,
    message: ServiceRequest<'_, StreamFileRequest>,
) -> ServiceResult<ServiceStream<StreamFileResponse>> {
    let identity = request::query_identity(
        &ctx,
        &service.authenticator,
        "/hephaestus.repository_browser.v1.RepositoryBrowserService/StreamFile",
    )
    .map_err(into_connect_error)?;
    let request = message.to_owned_message();
    let id = parse_id(request.repository_id.as_option()).map_err(into_connect_error)?;
    let total = if request.max_total_bytes == 0 {
        DEFAULT_TOTAL
    } else {
        usize::try_from(request.max_total_bytes)
            .map_err(|_| into_connect_error(RpcError::InvalidArgument))?
    };
    let chunk = if request.max_chunk_bytes == 0 {
        DEFAULT_CHUNK
    } else {
        usize::try_from(request.max_chunk_bytes)
            .map_err(|_| into_connect_error(RpcError::InvalidArgument))?
    };
    if total > MAX_TOTAL || chunk == 0 || chunk > MAX_CHUNK {
        return Err(into_connect_error(RpcError::InvalidArgument));
    }
    let budget = request::RequestBudget::from_transport(&ctx);
    let (selected, entry, contents) = request::run_with_budget(
        &budget,
        service
            .application
            .blob(&identity, id, &request.branch, &request.path, MAX_TOTAL),
    )
    .await
    .map_err(into_connect_error)?
    .map_err(map_error)
    .map_err(into_connect_error)?;
    let offset = parse_cursor(
        request
            .resume_cursor
            .as_option()
            .map(|cursor| cursor.value.as_str()),
        &selected.commit,
        &entry.object_id,
        contents.len(),
    )
    .map_err(into_connect_error)?;
    let end = offset.saturating_add(total).min(contents.len());
    let media_type = media_type(&request.path);
    let mut responses = contents[offset..end]
        .chunks(chunk)
        .enumerate()
        .map(|(index, bytes)| {
            let committed = offset + ((index + 1) * chunk).min(end - offset);
            Ok(StreamFileResponse {
                sequence: u64::try_from(index).unwrap_or(u64::MAX),
                contents: bytes.to_vec(),
                committed_cursor: Cursor {
                    value: format!("v1:{}:{}:{committed}", selected.commit, entry.object_id),
                    ..Default::default()
                }
                .into(),
                end_of_file: committed == contents.len(),
                media_type: String::from(media_type),
                ..Default::default()
            })
        })
        .collect::<Vec<Result<_, connectrpc::ConnectError>>>();
    if responses.is_empty() && end == contents.len() {
        responses.push(Ok(StreamFileResponse {
            committed_cursor: Cursor {
                value: format!("v1:{}:{}:{end}", selected.commit, entry.object_id),
                ..Default::default()
            }
            .into(),
            end_of_file: true,
            media_type: String::from(media_type),
            ..Default::default()
        }));
    }
    Response::ok(response_stream(responses, budget, ctx.deadline()))
}

fn response_stream(
    responses: Vec<Result<StreamFileResponse, connectrpc::ConnectError>>,
    budget: request::RequestBudget,
    deadline: Option<Instant>,
) -> ServiceStream<StreamFileResponse> {
    let (sender, receiver) = mpsc::channel(2);
    let cancellation = budget.cancellation_token();
    tokio::spawn(produce(sender, responses, cancellation, deadline));
    let response = stream::unfold(
        (receiver, budget, false),
        |(mut receiver, budget, terminal)| async move {
            if terminal {
                return None;
            }
            match request::run_with_budget(&budget, receiver.recv()).await {
                Ok(Some(Ok(response))) => Some((Ok(response), (receiver, budget, false))),
                Ok(Some(Err(error))) => {
                    let terminal = error.code == connectrpc::ErrorCode::DeadlineExceeded;
                    if terminal {
                        budget.cancel();
                    }
                    Some((Err(error), (receiver, budget, terminal)))
                }
                Err(RpcError::DeadlineExceeded) => {
                    budget.cancel();
                    Some((
                        Err(into_connect_error(RpcError::DeadlineExceeded)),
                        (receiver, budget, true),
                    ))
                }
                Err(_) | Ok(None) => None,
            }
        },
    );
    Box::pin(response)
}

async fn produce(
    sender: mpsc::Sender<Result<StreamFileResponse, connectrpc::ConnectError>>,
    responses: Vec<Result<StreamFileResponse, connectrpc::ConnectError>>,
    cancellation: CancellationToken,
    deadline: Option<Instant>,
) {
    for response in responses {
        match send_with_budget(&sender, response, &cancellation, deadline).await {
            Ok(()) => {}
            Err(Boundary::Deadline) => {
                let _ = sender.try_send(Err(into_connect_error(RpcError::DeadlineExceeded)));
                cancellation.cancel();
                return;
            }
            Err(Boundary::Closed | Boundary::Canceled) => return,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Boundary {
    Closed,
    Canceled,
    Deadline,
}

async fn send_with_budget(
    sender: &mpsc::Sender<Result<StreamFileResponse, connectrpc::ConnectError>>,
    response: Result<StreamFileResponse, connectrpc::ConnectError>,
    cancellation: &CancellationToken,
    deadline: Option<Instant>,
) -> Result<(), Boundary> {
    if cancellation.is_cancelled() {
        return Err(Boundary::Canceled);
    }
    if deadline.is_some_and(|value| Instant::now() >= value) {
        cancellation.cancel();
        return Err(Boundary::Deadline);
    }
    let send = sender.send(response);
    match deadline {
        Some(deadline) => {
            tokio::select! {
                result = send => if result.is_ok() {
                    Ok(())
                } else {
                    cancellation.cancel();
                    Err(Boundary::Closed)
                },
                () = cancellation.cancelled() => Err(Boundary::Canceled),
                () = sleep_until(deadline.into()) => {
                    cancellation.cancel();
                    Err(Boundary::Deadline)
                }
            }
        }
        None => {
            tokio::select! {
                result = send => if result.is_ok() {
                    Ok(())
                } else {
                    cancellation.cancel();
                    Err(Boundary::Closed)
                },
                () = cancellation.cancelled() => Err(Boundary::Canceled),
            }
        }
    }
}

fn parse_cursor(
    value: Option<&str>,
    commit: &str,
    object: &str,
    length: usize,
) -> Result<usize, RpcError> {
    let Some(value) = value.filter(|value| !value.is_empty()) else {
        return Ok(0);
    };
    let value = value.strip_prefix("v1:").ok_or(RpcError::InvalidArgument)?;
    let mut fields = value.split(':');
    let cursor_commit = fields.next().ok_or(RpcError::InvalidArgument)?;
    let cursor_object = fields.next().ok_or(RpcError::InvalidArgument)?;
    let offset: usize = fields
        .next()
        .ok_or(RpcError::InvalidArgument)?
        .parse()
        .map_err(|_| RpcError::InvalidArgument)?;
    if fields.next().is_some()
        || cursor_commit != commit
        || cursor_object != object
        || offset > length
    {
        return Err(RpcError::InvalidArgument);
    }
    Ok(offset)
}

fn media_type(path: &str) -> &'static str {
    match std::path::Path::new(path)
        .extension()
        .and_then(std::ffi::OsStr::to_str)
    {
        Some("json") => "application/json",
        Some("md" | "txt" | "rs" | "ex" | "exs" | "toml" | "sql" | "sh" | "yml" | "yaml") => {
            "text/plain; charset=utf-8"
        }
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::{Boundary, parse_cursor, send_with_budget};
    use rpc_proto::messages::hephaestus::repository_browser::v1::StreamFileResponse;
    use std::time::{Duration, Instant};
    use tokio::sync::mpsc;
    #[test]
    fn cursor_is_bound_to_commit_object_and_length() {
        assert_eq!(
            parse_cursor(Some("v1:abc:def:4"), "abc", "def", 8).expect("cursor"),
            4
        );
        assert!(parse_cursor(Some("v1:other:def:4"), "abc", "def", 8).is_err());
        assert!(parse_cursor(Some("v1:abc:def:9"), "abc", "def", 8).is_err());
    }

    #[tokio::test]
    async fn producer_cancels_when_receiver_drops() {
        let cancellation = tokio_util::sync::CancellationToken::new();
        let (sender, receiver) = mpsc::channel(1);
        drop(receiver);
        let result = send_with_budget(
            &sender,
            Ok(StreamFileResponse::default()),
            &cancellation,
            None,
        )
        .await;
        assert_eq!(result, Err(Boundary::Closed));
        assert!(cancellation.is_cancelled());
    }

    #[tokio::test]
    async fn producer_deadline_wins_before_ready_send() {
        let cancellation = tokio_util::sync::CancellationToken::new();
        let (sender, _receiver) = mpsc::channel(1);
        let result = send_with_budget(
            &sender,
            Ok(StreamFileResponse::default()),
            &cancellation,
            Some(
                Instant::now()
                    .checked_sub(Duration::from_millis(1))
                    .expect("instant supports subtraction"),
            ),
        )
        .await;
        assert_eq!(result, Err(Boundary::Deadline));
        assert!(cancellation.is_cancelled());
    }
}
