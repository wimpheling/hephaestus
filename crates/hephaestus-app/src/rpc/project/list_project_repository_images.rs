use super::{ProjectRpc, map_error, opaque, parse_id, parse_page, timestamp};
use crate::rpc::{into_connect_error, request};
use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult};
use rpc_proto::messages::hephaestus::{
    common::v1::PageResponse,
    project::v1::{
        ListProjectRepositoryImagesRequest, ListProjectRepositoryImagesResponse,
        ProjectRepositoryImage,
    },
};

pub(super) async fn handle(
    service: &ProjectRpc,
    ctx: RequestContext,
    message: ServiceRequest<'_, ListProjectRepositoryImagesRequest>,
) -> ServiceResult<ListProjectRepositoryImagesResponse> {
    let identity = request::query_identity(
        &ctx,
        &service.authenticator,
        "/hephaestus.project.v1.ProjectService/ListProjectRepositoryImages",
    )
    .map_err(into_connect_error)?;
    let request = message.to_owned_message();
    let project_id = parse_id(request.project_id.as_option()).map_err(into_connect_error)?;
    let page = parse_page(request.page.as_option()).map_err(into_connect_error)?;
    let result = service
        .application
        .repository_images(&identity, project_id, page)
        .await
        .map_err(map_error)
        .map_err(into_connect_error)?;
    Response::ok(ListProjectRepositoryImagesResponse {
        images: result
            .values
            .into_iter()
            .map(|row| ProjectRepositoryImage {
                id: opaque(row.id).into(),
                repository_id: opaque(row.repository_id).into(),
                key: row.key,
                display_name: row.display_name,
                source_revision: row.source_revision,
                base_image_reference: row.base_image_reference,
                status: row.status,
                image_reference: row.image_reference.unwrap_or_default(),
                failure_reason: row.failure_reason.unwrap_or_default(),
                updated_at: timestamp(row.updated_at).into(),
                ..Default::default()
            })
            .collect(),
        page: PageResponse {
            next_page_token: result.next.unwrap_or_default(),
            stable_order: String::from("id"),
            ..Default::default()
        }
        .into(),
        ..Default::default()
    })
}
