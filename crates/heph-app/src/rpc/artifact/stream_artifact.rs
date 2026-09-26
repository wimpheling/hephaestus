use std::{future::Future, pin::Pin, sync::Arc};

use connectrpc::ConnectError;
use control_plane_postgres::artifact::{
    ArtifactCancellation, ArtifactChunk, ArtifactError, ArtifactStream,
};
use rpc_proto::messages::hephaestus::{artifact::v1::StreamArtifactResponse, common::v1::Cursor};
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
struct RequestCancellation(CancellationToken);

impl ArtifactCancellation for RequestCancellation {
    fn is_cancelled(&self) -> bool {
        self.0.is_cancelled()
    }

    fn cancel(&self) {
        self.0.cancel();
    }

    fn cancelled<'a>(&'a self) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(self.0.cancelled())
    }
}

enum ResponseStreamState {
    Starting {
        operation: Pin<Box<dyn Future<Output = Result<ArtifactStream, ArtifactError>> + Send>>,
        budget: super::super::request::RequestBudget,
    },
    Reading {
        receiver: tokio::sync::mpsc::Receiver<Result<ArtifactChunk, ArtifactError>>,
        budget: super::super::request::RequestBudget,
        deadline_sent: bool,
    },
    Finished,
}

pub(super) async fn handle(
    service: &super::ArtifactRpc,
    ctx: &connectrpc::RequestContext,
    request: connectrpc::ServiceRequest<
        '_,
        rpc_proto::messages::hephaestus::artifact::v1::StreamArtifactRequest,
    >,
) -> connectrpc::ServiceResult<
    connectrpc::ServiceStream<
        rpc_proto::messages::hephaestus::artifact::v1::StreamArtifactResponse,
    >,
> {
    use crate::application::artifact::StreamArtifact;
    use uuid::Uuid;

    use super::super::{RpcError, into_connect_error, request as shared_request};

    const AUDIENCE: &str = "/hephaestus.artifact.v1.ArtifactService/StreamArtifact";

    let identity = shared_request::query_identity(ctx, &service.authenticator, AUDIENCE)
        .map_err(into_connect_error)?;
    let request = request.to_owned_message();
    let artifact_id = shared_request::required_id(request.artifact_id.as_option())
        .and_then(|value| Uuid::parse_str(&value).map_err(|_| RpcError::InvalidArgument))
        .map_err(into_connect_error)?;
    let budget = shared_request::RequestBudget::from_transport(ctx);
    let request = StreamArtifact {
        artifact_id,
        resume_cursor: request
            .resume_cursor
            .as_option()
            .filter(|cursor| !cursor.value.is_empty())
            .map(|cursor| cursor.value.clone()),
        max_total_bytes: request.max_total_bytes,
        max_chunk_bytes: request.max_chunk_bytes,
    };
    let deadline = ctx.deadline();
    let operation = Box::pin({
        let application = Arc::clone(&service.application);
        let cancellation = RequestCancellation(budget.cancellation_token());
        async move {
            application
                .stream_artifact_with_budget(&identity, request, cancellation, deadline)
                .await
        }
    });
    let response = response_stream(operation, budget);
    // The generated service contract and RPC architecture check require an awaited handler.
    std::future::ready(()).await;
    connectrpc::Response::ok(Box::pin(response))
}

fn response_stream(
    operation: Pin<Box<dyn Future<Output = Result<ArtifactStream, ArtifactError>> + Send>>,
    budget: super::super::request::RequestBudget,
) -> impl futures_util::Stream<Item = Result<StreamArtifactResponse, ConnectError>> + Send {
    futures_util::stream::unfold(
        ResponseStreamState::Starting { operation, budget },
        next_stream_item,
    )
}

async fn next_stream_item(
    state: ResponseStreamState,
) -> Option<(
    Result<StreamArtifactResponse, ConnectError>,
    ResponseStreamState,
)> {
    match state {
        ResponseStreamState::Starting { operation, budget } => {
            match super::super::request::run_with_budget(&budget, operation).await {
                Ok(Ok(result)) => {
                    let mut receiver = result.receiver;
                    let (item, deadline_sent) =
                        next_response(&mut receiver, &budget, false).await?;
                    Some((
                        item,
                        ResponseStreamState::Reading {
                            receiver,
                            budget,
                            deadline_sent,
                        },
                    ))
                }
                Ok(Err(error)) => Some((
                    Err(super::super::into_connect_error(
                        super::model::application_error(error),
                    )),
                    ResponseStreamState::Finished,
                )),
                Err(error) => Some((
                    Err(super::super::into_connect_error(error)),
                    ResponseStreamState::Finished,
                )),
            }
        }
        ResponseStreamState::Reading {
            mut receiver,
            budget,
            deadline_sent,
        } => {
            let (item, deadline_sent) =
                next_response(&mut receiver, &budget, deadline_sent).await?;
            Some((
                item,
                ResponseStreamState::Reading {
                    receiver,
                    budget,
                    deadline_sent,
                },
            ))
        }
        ResponseStreamState::Finished => None,
    }
}

async fn next_response(
    receiver: &mut tokio::sync::mpsc::Receiver<Result<ArtifactChunk, ArtifactError>>,
    budget: &super::super::request::RequestBudget,
    deadline_sent: bool,
) -> Option<(Result<StreamArtifactResponse, ConnectError>, bool)> {
    if deadline_sent {
        return None;
    }
    match super::super::request::run_with_budget(budget, receiver.recv()).await {
        Ok(Some(Ok(chunk))) => Some((
            Ok(StreamArtifactResponse {
                sequence: chunk.sequence,
                contents: chunk.contents,
                committed_cursor: Cursor {
                    value: chunk.committed_cursor,
                    ..Default::default()
                }
                .into(),
                end_of_artifact: chunk.end_of_artifact,
                media_type: chunk.media_type,
                ..Default::default()
            }),
            false,
        )),
        Ok(Some(Err(ArtifactError::DeadlineExceeded)))
        | Err(super::super::RpcError::DeadlineExceeded) => {
            budget.cancel();
            Some((
                Err(super::super::into_connect_error(
                    super::super::RpcError::DeadlineExceeded,
                )),
                true,
            ))
        }
        Ok(Some(Err(error))) => Some((
            Err(super::super::into_connect_error(
                super::model::application_error(error),
            )),
            false,
        )),
        Err(_) | Ok(None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::next_response;
    use crate::rpc::request::RequestBudget;
    use control_plane_postgres::artifact::{ArtifactChunk, ArtifactError};
    use std::time::{Duration, Instant};
    use tokio::sync::mpsc;

    #[tokio::test]
    async fn response_stream_emits_one_deadline_after_channel_is_empty() {
        let budget = RequestBudget::from_deadline(Some(
            Instant::now()
                .checked_sub(Duration::from_millis(1))
                .expect("instant supports subtraction"),
        ));
        let (sender, mut receiver) = mpsc::channel::<Result<ArtifactChunk, ArtifactError>>(1);
        let (first, deadline_sent) = next_response(&mut receiver, &budget, false)
            .await
            .expect("deadline response");
        assert_eq!(
            first.expect_err("deadline must be an error").code,
            connectrpc::ErrorCode::DeadlineExceeded
        );
        assert!(
            next_response(&mut receiver, &budget, deadline_sent)
                .await
                .is_none()
        );
        drop(sender);
    }

    #[tokio::test]
    async fn response_stream_treats_adapter_deadline_as_terminal() {
        let budget = RequestBudget::unbounded();
        let (sender, mut receiver) = mpsc::channel::<Result<ArtifactChunk, ArtifactError>>(1);
        sender
            .send(Err(ArtifactError::DeadlineExceeded))
            .await
            .expect("receiver is alive");

        let (first, deadline_sent) = next_response(&mut receiver, &budget, false)
            .await
            .expect("deadline response");
        assert_eq!(
            first.expect_err("deadline must be an error").code,
            connectrpc::ErrorCode::DeadlineExceeded
        );
        assert!(
            next_response(&mut receiver, &budget, deadline_sent)
                .await
                .is_none()
        );
    }
}
