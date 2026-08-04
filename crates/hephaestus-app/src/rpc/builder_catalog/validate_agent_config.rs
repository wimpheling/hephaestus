use super::{
    BuilderCatalogRpc, VALIDATE_AUDIENCE, map_catalog_error, map_error, to_proto_with_registry,
};
use crate::rpc::{into_connect_error, request};
use agent_config::parse;
use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult};
use rpc_proto::messages::hephaestus::builder::v1::{
    ValidateAgentConfigRequest, ValidateAgentConfigResponse,
};

pub(super) async fn handle(
    service: &BuilderCatalogRpc,
    ctx: RequestContext,
    request_message: ServiceRequest<'_, ValidateAgentConfigRequest>,
) -> ServiceResult<ValidateAgentConfigResponse> {
    let identity = request::query_identity(&ctx, &service.authenticator, VALIDATE_AUDIENCE)
        .map_err(into_connect_error)?;
    let parsed = parse(&request_message.to_owned_message().agent_toml);
    let config = parsed
        .config
        .ok_or(crate::rpc::RpcError::InvalidArgument)
        .map_err(into_connect_error)?;
    let selection = service
        .application
        .validate_agent_config(&config)
        .await
        .map_err(map_error)
        .map_err(into_connect_error)?;
    let image = service
        .application
        .get_builder_image_publication(&identity, selection.image_id)
        .await
        .map_err(map_catalog_error)
        .map_err(into_connect_error)?;
    Response::ok(ValidateAgentConfigResponse {
        builder_image: to_proto_with_registry(image.image, image.registry_publication).into(),
        network: super::network(selection.network).into(),
        vcpus: u32::from(selection.vcpus),
        memory_mib: selection.memory_mib,
        platform_policy_version: selection.platform_policy_version,
        ..Default::default()
    })
}
