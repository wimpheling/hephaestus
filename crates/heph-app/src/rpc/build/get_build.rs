pub(super) async fn handle(
    service: &super::BuildRpc,
    ctx: connectrpc::RequestContext,
    request: connectrpc::ServiceRequest<
        '_,
        rpc_proto::messages::hephaestus::build::v1::GetBuildRequest,
    >,
) -> connectrpc::ServiceResult<rpc_proto::messages::hephaestus::build::v1::GetBuildResponse> {
    use super::super::{into_connect_error, request as shared_request};
    use rpc_proto::messages::hephaestus::build::v1::GetBuildResponse;
    use uuid::Uuid;

    const AUDIENCE: &str = "/hephaestus.build.v1.BuildService/GetBuild";

    let identity = shared_request::query_identity(&ctx, &service.authenticator, AUDIENCE)
        .map_err(into_connect_error)?;
    let request = request.to_owned_message();
    let id = shared_request::required_id(request.build_id.as_option())
        .and_then(|value| {
            Uuid::parse_str(&value).map_err(|_| super::super::RpcError::InvalidArgument)
        })
        .map_err(into_connect_error)?;
    let result = service
        .application
        .get_build(&identity, id)
        .await
        .map_err(super::model::application_error)
        .map_err(into_connect_error)?;
    connectrpc::Response::ok(GetBuildResponse {
        build: super::model::build(result).into(),
        ..Default::default()
    })
}
