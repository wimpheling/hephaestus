use super::{RepositoryRpc, map_forge_error, opaque};
use crate::rpc::{RpcError, into_connect_error, request};
use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult};
use forge_domain::{GitRef, ProjectId};
use forge_service::CreateRepository;
use rpc_proto::messages::hephaestus::repository::v1::{
    CreateRepositoryRequest, CreateRepositoryResponse,
};

pub(super) async fn handle(
    service: &RepositoryRpc,
    ctx: RequestContext,
    message: ServiceRequest<'_, CreateRepositoryRequest>,
) -> ServiceResult<CreateRepositoryResponse> {
    let request = message.to_owned_message();
    let identity = request::mutation_identity(
        &ctx,
        &service.authenticator,
        "/hephaestus.repository.v1.RepositoryService/CreateRepository",
        request.context.as_option(),
    )
    .map_err(into_connect_error)?;
    let project_id = request
        .project_id
        .as_option()
        .ok_or_else(|| into_connect_error(RpcError::InvalidArgument))?
        .value
        .parse::<ProjectId>()
        .map_err(|_| into_connect_error(RpcError::InvalidArgument))?;
    let default_branch = if request.default_branch.starts_with("refs/") {
        request.default_branch
    } else {
        format!("refs/heads/{}", request.default_branch)
    };
    let default_branch =
        GitRef::parse(default_branch).map_err(|_| into_connect_error(RpcError::InvalidArgument))?;
    let repository = service
        .forge
        .create_repository(
            &identity,
            &CreateRepository {
                project_id,
                name: request.name,
                default_branch,
                is_public: request.is_public,
                agent_runs_enabled: request.agent_runs_enabled,
            },
        )
        .await
        .map_err(|error| map_forge_error(&error))
        .map_err(into_connect_error)?;
    let receipt = crate::rpc::mutation_receipt(
        &service.receipts,
        identity.idempotency_id,
        identity.user_id,
        "repository",
        "repository",
    )
    .await?;
    Response::ok(CreateRepositoryResponse {
        repository_id: opaque(repository.id.as_uuid()).into(),
        receipt: receipt.into(),
        ..Default::default()
    })
}
