use super::{ProjectRpc, map_error, opaque, timestamp};
use crate::rpc::{into_connect_error, mutation_receipt, request};
use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult};
use rpc_proto::messages::hephaestus::project::v1::{
    ProjectRepositoryImage, RetryProjectRepositoryImageRequest, RetryProjectRepositoryImageResponse,
};
use uuid::Uuid;

pub(super) async fn handle(
    service: &ProjectRpc,
    ctx: RequestContext,
    message: ServiceRequest<'_, RetryProjectRepositoryImageRequest>,
) -> ServiceResult<RetryProjectRepositoryImageResponse> {
    let request = message.to_owned_message();
    let identity = request::mutation_identity(
        &ctx,
        &service.authenticator,
        "/hephaestus.project.v1.ProjectService/RetryProjectRepositoryImage",
        request.context.as_option(),
    )
    .map_err(into_connect_error)?;
    let image_id = request
        .image_id
        .as_option()
        .ok_or_else(|| into_connect_error(crate::rpc::RpcError::InvalidArgument))?
        .value
        .parse::<Uuid>()
        .map_err(|_| into_connect_error(crate::rpc::RpcError::InvalidArgument))?;
    let image = service
        .application
        .retry_repository_image(&identity, image_id)
        .await
        .map_err(map_error)
        .map_err(into_connect_error)?;
    let receipt = mutation_receipt(
        &service.receipts,
        identity.idempotency_id,
        identity.user_id,
        "project",
        "project",
    )
    .await?;
    Response::ok(RetryProjectRepositoryImageResponse {
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
        receipt: receipt.into(),
        ..Default::default()
    })
}
