pub(super) async fn handle(
    service: &super::ArtifactRpc,
    ctx: connectrpc::RequestContext,
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
    use futures_util::stream;
    use rpc_proto::messages::hephaestus::{
        artifact::v1::StreamArtifactResponse, common::v1::Cursor,
    };
    use uuid::Uuid;

    use super::super::{RpcError, into_connect_error, request as shared_request};

    const AUDIENCE: &str = "/hephaestus.artifact.v1.ArtifactService/StreamArtifact";

    let identity = shared_request::query_identity(&ctx, &service.authenticator, AUDIENCE)
        .map_err(into_connect_error)?;
    let request = request.to_owned_message();
    let artifact_id = shared_request::required_id(request.artifact_id.as_option())
        .and_then(|value| Uuid::parse_str(&value).map_err(|_| RpcError::InvalidArgument))
        .map_err(into_connect_error)?;
    let result = service
        .application
        .stream_artifact(
            &identity,
            StreamArtifact {
                artifact_id,
                resume_cursor: request
                    .resume_cursor
                    .as_option()
                    .filter(|cursor| !cursor.value.is_empty())
                    .map(|cursor| cursor.value.clone()),
                max_total_bytes: request.max_total_bytes,
                max_chunk_bytes: request.max_chunk_bytes,
            },
        )
        .await
        .map_err(super::model::application_error)
        .map_err(into_connect_error)?;
    let receiver = result.receiver;
    let response = stream::unfold(receiver, |mut receiver| async move {
        let item = receiver.recv().await?;
        let item = item
            .map(|chunk| StreamArtifactResponse {
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
            })
            .map_err(super::model::application_error)
            .map_err(into_connect_error);
        Some((item, receiver))
    });
    connectrpc::Response::ok(Box::pin(response))
}
