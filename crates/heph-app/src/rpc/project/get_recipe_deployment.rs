//! Authenticated persisted progress inspection without execution authority.

mod projection;
#[cfg(test)]
mod tests;

use super::{ProjectRpc, parse_id};
use crate::rpc::{RpcError, into_connect_error, request};
use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult};
use recipe_application::DeploymentId;
use rpc_proto::messages::hephaestus::project::v1::{
    GetRecipeDeploymentRequest, GetRecipeDeploymentResponse,
};

const AUDIENCE: &str = "/hephaestus.project.v1.ProjectService/GetRecipeDeployment";

pub(super) async fn handle(
    service: &ProjectRpc,
    ctx: RequestContext,
    message: ServiceRequest<'_, GetRecipeDeploymentRequest>,
) -> ServiceResult<GetRecipeDeploymentResponse> {
    let identity = request::query_identity(&ctx, &service.authenticator, AUDIENCE)
        .map_err(into_connect_error)?;
    let budget = request::RequestBudget::from_transport(&ctx);
    let input = message.to_owned_message();
    let project = parse_id(input.project_id.as_option()).map_err(into_connect_error)?;
    if project.is_nil() {
        return Err(into_connect_error(RpcError::InvalidArgument));
    }
    let deployment = DeploymentId::from_uuid(
        parse_id(input.deployment_id.as_option()).map_err(into_connect_error)?,
    )
    .map_err(|_| into_connect_error(RpcError::InvalidArgument))?;
    // This retained application-role repository independently checks current
    // CanManage on the original project; the query's project.read tag is no grant.
    let snapshot =
        request::run_with_budget(&budget, service.recipes.inspect(&identity, deployment))
            .await
            .map_err(into_connect_error)?
            .map_err(|error| map_error(&error))
            .map_err(into_connect_error)?;
    let deployment =
        projection::project(&snapshot, project, deployment).map_err(into_connect_error)?;
    Response::ok(GetRecipeDeploymentResponse {
        deployment: deployment.into(),
        ..Default::default()
    })
}

const fn map_error(error: &recipe_application::DeploymentError) -> RpcError {
    use recipe_application::DeploymentError;
    match error {
        DeploymentError::AuthorizationDenied => RpcError::PermissionDenied,
        DeploymentError::Unavailable | DeploymentError::Repository => RpcError::Unavailable,
        _ => RpcError::FailedPrecondition,
    }
}
