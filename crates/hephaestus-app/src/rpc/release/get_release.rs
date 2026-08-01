pub(super) async fn handle(
    service: &super::ReleaseRpc,
    ctx: connectrpc::RequestContext,
    request: connectrpc::ServiceRequest<
        '_,
        rpc_proto::messages::hephaestus::release::v1::GetReleaseRequest,
    >,
) -> connectrpc::ServiceResult<rpc_proto::messages::hephaestus::release::v1::GetReleaseResponse> {
    use rpc_proto::messages::hephaestus::release::v1::GetReleaseResponse;
    use uuid::Uuid;

    use super::super::{RpcError, into_connect_error, request as shared_request};

    const AUDIENCE: &str = "/hephaestus.release.v1.ReleaseService/GetRelease";

    let identity = shared_request::query_identity(&ctx, &service.authenticator, AUDIENCE)
        .map_err(into_connect_error)?;
    let request = request.to_owned_message();
    let id = shared_request::required_id(request.release_id.as_option())
        .and_then(|value| Uuid::parse_str(&value).map_err(|_| RpcError::InvalidArgument))
        .map_err(into_connect_error)?;
    let result = service
        .application
        .get_release(&identity, id)
        .await
        .map_err(super::model::application_error)
        .map_err(into_connect_error)?;
    connectrpc::Response::ok(GetReleaseResponse {
        release: super::model::release(result).into(),
        ..Default::default()
    })
}
