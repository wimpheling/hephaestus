use super::{ProjectRpc, map_forge_error, opaque};
use crate::rpc::{into_connect_error, request};
use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult};
use forge_domain::OrganizationId;
use rpc_proto::messages::hephaestus::project::v1::{CreateProjectRequest, CreateProjectResponse};

pub(super) async fn handle(
    service: &ProjectRpc,
    ctx: RequestContext,
    message: ServiceRequest<'_, CreateProjectRequest>,
) -> ServiceResult<CreateProjectResponse> {
    let request = message.to_owned_message();
    let identity = request::mutation_identity(
        &ctx,
        &service.authenticator,
        "/hephaestus.project.v1.ProjectService/CreateProject",
        request.context.as_option(),
    )
    .map_err(into_connect_error)?;
    let organization_id = request
        .organization_id
        .as_option()
        .ok_or_else(|| into_connect_error(crate::rpc::RpcError::InvalidArgument))?
        .value
        .parse::<OrganizationId>()
        .map_err(|_| into_connect_error(crate::rpc::RpcError::InvalidArgument))?;
    let project = service
        .forge
        .create_project_with_description(
            &identity,
            organization_id,
            &request.name,
            &request.description,
        )
        .await
        .map_err(|error| map_forge_error(&error))
        .map_err(into_connect_error)?;
    let receipt = crate::rpc::mutation_receipt(
        &service.receipts,
        identity.idempotency_id,
        identity.user_id,
        "project",
        "project",
    )
    .await?;
    Response::ok(CreateProjectResponse {
        project_id: opaque(project.id.as_uuid()).into(),
        receipt: receipt.into(),
        ..Default::default()
    })
}
