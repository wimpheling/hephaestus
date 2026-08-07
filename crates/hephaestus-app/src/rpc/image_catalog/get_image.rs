use super::{GET_AUDIENCE, ImageCatalogRpc, map_catalog_error, to_proto_with_registry};
use crate::rpc::{into_connect_error, request};
use builder_catalog_domain::OciImageId;
use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult};
use rpc_proto::messages::hephaestus::image::v1::{GetImageRequest, GetImageResponse};
use std::str::FromStr;
use uuid::Uuid;

pub(super) async fn handle(
    service: &ImageCatalogRpc,
    ctx: RequestContext,
    request_message: ServiceRequest<'_, GetImageRequest>,
) -> ServiceResult<GetImageResponse> {
    let identity = request::query_identity(&ctx, &service.authenticator, GET_AUDIENCE)
        .map_err(into_connect_error)?;
    let id = request_message
        .to_owned_message()
        .image_id
        .as_option()
        .ok_or(crate::rpc::RpcError::InvalidArgument)
        .and_then(|value| {
            Uuid::from_str(&value.value).map_err(|_| crate::rpc::RpcError::InvalidArgument)
        })
        .map_err(into_connect_error)?;
    let image = service
        .application
        .get_image_publication(&identity, OciImageId::from_uuid(id))
        .await
        .map_err(map_catalog_error)
        .map_err(into_connect_error)?;
    Response::ok(GetImageResponse {
        image: to_proto_with_registry(image.image, image.registry_publication).into(),
        ..Default::default()
    })
}
