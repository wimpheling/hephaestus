use super::{
    DEFAULT_PAGE_SIZE, ImageCatalogRpc, LIST_AUDIENCE, MAX_PAGE_SIZE, map_catalog_error,
    to_proto_with_registry,
};
use crate::rpc::{into_connect_error, request};
use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult};
use rpc_proto::messages::hephaestus::{
    common::v1::PageResponse,
    image::v1::{ListImagesRequest, ListImagesResponse},
};

pub(super) async fn handle(
    service: &ImageCatalogRpc,
    ctx: RequestContext,
    request_message: ServiceRequest<'_, ListImagesRequest>,
) -> ServiceResult<ListImagesResponse> {
    let identity = request::query_identity(&ctx, &service.authenticator, LIST_AUDIENCE)
        .map_err(into_connect_error)?;
    let request = request_message.to_owned_message();
    let page_size = request
        .page
        .as_option()
        .map_or(DEFAULT_PAGE_SIZE, |page| page.page_size.max(1));
    if page_size > MAX_PAGE_SIZE {
        return Err(into_connect_error(crate::rpc::RpcError::InvalidArgument));
    }
    let offset = request
        .page
        .as_option()
        .and_then(|page| (!page.page_token.is_empty()).then_some(page.page_token.as_str()))
        .map_or(Ok(0_usize), |token| {
            token
                .parse::<usize>()
                .map_err(|_| crate::rpc::RpcError::InvalidArgument)
        })
        .map_err(into_connect_error)?;
    let images = service
        .application
        .list_image_publications(&identity)
        .await
        .map_err(map_catalog_error)
        .map_err(into_connect_error)?;
    let page_size = usize::try_from(page_size).expect("bounded page size fits usize");
    let mut images = images.into_iter().skip(offset).collect::<Vec<_>>();
    let has_more = images.len() > page_size;
    if has_more {
        images.truncate(page_size);
    }
    Response::ok(ListImagesResponse {
        images: images
            .into_iter()
            .map(|publication| {
                to_proto_with_registry(publication.image, publication.registry_publication)
            })
            .collect(),
        page: PageResponse {
            next_page_token: if has_more {
                (offset + page_size).to_string()
            } else {
                String::new()
            },
            stable_order: String::from("key,id"),
            ..Default::default()
        }
        .into(),
        ..Default::default()
    })
}
