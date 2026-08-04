use super::{BuilderCatalogRpc, GET_AUDIENCE, map_catalog_error, to_proto_with_registry};
use crate::rpc::{into_connect_error, request};
use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult};
use rpc_proto::messages::hephaestus::builder::v1::{
    GetBuilderImageRequest, GetBuilderImageResponse,
};
use std::str::FromStr;
use uuid::Uuid;

pub(super) async fn handle(
    service: &BuilderCatalogRpc,
    ctx: RequestContext,
    request_message: ServiceRequest<'_, GetBuilderImageRequest>,
) -> ServiceResult<GetBuilderImageResponse> {
    let identity = request::query_identity(&ctx, &service.authenticator, GET_AUDIENCE)
        .map_err(into_connect_error)?;
    let id = request_message
        .to_owned_message()
        .builder_image_id
        .as_option()
        .ok_or(crate::rpc::RpcError::InvalidArgument)
        .and_then(|value| {
            Uuid::from_str(&value.value).map_err(|_| crate::rpc::RpcError::InvalidArgument)
        })
        .map_err(into_connect_error)?;
    let image = service
        .application
        .get_builder_image_publication(
            &identity,
            builder_catalog_domain::BuilderImageId::from_uuid(id),
        )
        .await
        .map_err(map_catalog_error)
        .map_err(into_connect_error)?;
    Response::ok(GetBuilderImageResponse {
        builder_image: to_proto_with_registry(image.image, image.registry_publication).into(),
        ..Default::default()
    })
}
