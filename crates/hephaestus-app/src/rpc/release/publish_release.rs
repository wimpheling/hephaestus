pub(super) async fn handle(
    service: &super::ReleaseRpc,
    ctx: connectrpc::RequestContext,
    request: connectrpc::ServiceRequest<
        '_,
        rpc_proto::messages::hephaestus::release::v1::PublishReleaseRequest,
    >,
) -> connectrpc::ServiceResult<rpc_proto::messages::hephaestus::release::v1::PublishReleaseResponse>
{
    use rpc_proto::messages::hephaestus::release::v1::PublishReleaseResponse;
    use uuid::Uuid;

    use super::super::{into_connect_error, request as shared_request};

    const AUDIENCE: &str = "/hephaestus.release.v1.ReleaseService/PublishRelease";

    let request = request.to_owned_message();
    let identity = shared_request::mutation_identity(
        &ctx,
        &service.authenticator,
        AUDIENCE,
        request.context.as_option(),
    )
    .map_err(into_connect_error)?;
    let release_id = shared_request::required_id(request.release_id.as_option())
        .and_then(|value| {
            Uuid::parse_str(&value).map_err(|_| super::super::RpcError::InvalidArgument)
        })
        .map_err(into_connect_error)?;
    service
        .application
        .publish_release(&identity, release_id)
        .await
        .map_err(super::model::application_error)
        .map_err(into_connect_error)?;
    let release = service
        .application
        .get_release(&identity, release_id)
        .await
        .map_err(super::model::application_error)
        .map_err(into_connect_error)?;
    let receipt = crate::rpc::mutation_receipt(
        &service.receipts,
        identity.idempotency_id,
        identity.user_id,
        "release",
        "repository",
    )
    .await?;
    connectrpc::Response::ok(PublishReleaseResponse {
        release: super::model::release(release).into(),
        receipt: receipt.into(),
        ..Default::default()
    })
}
