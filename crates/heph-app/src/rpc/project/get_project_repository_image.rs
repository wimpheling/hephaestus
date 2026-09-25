use super::{ProjectRpc, map_error, opaque, parse_page, timestamp};
use crate::rpc::{into_connect_error, request};
use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult};
use rpc_proto::messages::hephaestus::{
    common::v1::PageResponse,
    project::v1::{
        GetProjectRepositoryImageRequest, GetProjectRepositoryImageResponse,
        ProjectRepositoryImage, ProjectRepositoryImagePreparationEvent,
    },
};
use uuid::Uuid;

pub(super) async fn handle(
    service: &ProjectRpc,
    ctx: RequestContext,
    message: ServiceRequest<'_, GetProjectRepositoryImageRequest>,
) -> ServiceResult<GetProjectRepositoryImageResponse> {
    let identity = request::query_identity(
        &ctx,
        &service.authenticator,
        "/hephaestus.project.v1.ProjectService/GetProjectRepositoryImage",
    )
    .map_err(into_connect_error)?;
    let request = message.to_owned_message();
    let image_id = request
        .image_id
        .as_option()
        .ok_or_else(|| into_connect_error(crate::rpc::RpcError::InvalidArgument))?
        .value
        .parse::<Uuid>()
        .map_err(|_| into_connect_error(crate::rpc::RpcError::InvalidArgument))?;
    let page = parse_page(request.page.as_option()).map_err(into_connect_error)?;
    let (image, history) = service
        .application
        .repository_image(&identity, image_id, page)
        .await
        .map_err(map_error)
        .map_err(into_connect_error)?;
    Response::ok(GetProjectRepositoryImageResponse {
        image: ProjectRepositoryImage {
            id: opaque(image.id).into(),
            repository_id: opaque(image.repository_id).into(),
            key: image.key,
            display_name: image.display_name,
            source_revision: image.source_revision,
            base_image_reference: image.base_image_reference,
            status: image.status,
            image_reference: image.image_reference.unwrap_or_default(),
            failure_reason: image.failure_reason.unwrap_or_default(),
            updated_at: timestamp(image.updated_at).into(),
            ..Default::default()
        }
        .into(),
        history: history
            .values
            .into_iter()
            .map(|event| ProjectRepositoryImagePreparationEvent {
                phase: event.phase,
                outcome: event.outcome,
                output_digest: event.output_digest.unwrap_or_default(),
                safe_reason: event.safe_reason.unwrap_or_default(),
                occurred_at: timestamp(event.occurred_at).into(),
                ..Default::default()
            })
            .collect(),
        page: PageResponse {
            next_page_token: history.next.unwrap_or_default(),
            stable_order: String::from("id"),
            ..Default::default()
        }
        .into(),
        ..Default::default()
    })
}
