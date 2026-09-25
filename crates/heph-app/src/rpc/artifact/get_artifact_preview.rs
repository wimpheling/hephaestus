pub(super) async fn handle(
    service: &super::ArtifactRpc,
    ctx: connectrpc::RequestContext,
    request: connectrpc::ServiceRequest<
        '_,
        rpc_proto::messages::hephaestus::artifact::v1::GetArtifactPreviewRequest,
    >,
) -> connectrpc::ServiceResult<
    rpc_proto::messages::hephaestus::artifact::v1::GetArtifactPreviewResponse,
> {
    use rpc_proto::messages::hephaestus::artifact::v1::GetArtifactPreviewResponse;
    use uuid::Uuid;

    use super::super::{RpcError, into_connect_error, request as shared_request};

    const AUDIENCE: &str = "/hephaestus.artifact.v1.ArtifactService/GetArtifactPreview";

    let identity = shared_request::query_identity(&ctx, &service.authenticator, AUDIENCE)
        .map_err(into_connect_error)?;
    let request = request.to_owned_message();
    let artifact_id = shared_request::required_id(request.artifact_id.as_option())
        .and_then(|value| Uuid::parse_str(&value).map_err(|_| RpcError::InvalidArgument))
        .map_err(into_connect_error)?;
    let result = service
        .application
        .get_artifact_preview(&identity, artifact_id, request.max_bytes)
        .await
        .map_err(super::model::application_error)
        .map_err(into_connect_error)?;
    connectrpc::Response::ok(GetArtifactPreviewResponse {
        artifact: super::model::artifact(&result.artifact).into(),
        utf8_contents: result.utf8_contents,
        truncated: result.truncated,
        ..Default::default()
    })
}
