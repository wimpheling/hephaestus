//! Transport waits only on registered node-owned work, never the engine future.
mod authentication;
mod input;
mod projection;
#[cfg(test)]
mod tests;

use super::ProjectRpc;
use crate::rpc::{RpcError, into_connect_error, request};
use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult};
use recipe_application::{CommandIdentity, DeploymentOperation};
use rpc_proto::messages::hephaestus::project::v1::{
    InstallRecipeDeploymentRequest, InstallRecipeDeploymentResponse, ResumeRecipeDeploymentRequest,
    ResumeRecipeDeploymentResponse,
};

const INSTALL: &str = "/hephaestus.project.v1.ProjectService/InstallRecipeDeployment";
const RESUME: &str = "/hephaestus.project.v1.ProjectService/ResumeRecipeDeployment";

pub(super) async fn install(
    service: &ProjectRpc,
    ctx: RequestContext,
    message: ServiceRequest<'_, InstallRecipeDeploymentRequest>,
) -> ServiceResult<InstallRecipeDeploymentResponse> {
    let input = message.to_owned_message();
    let identity = authentication::identity(
        &ctx,
        &service.authenticator,
        INSTALL,
        input.context.as_option(),
    )
    .map_err(into_connect_error)?;
    let plan = input::install(&input).map_err(into_connect_error)?;
    let project = plan.project_id().as_uuid();
    let deployment = plan.id();
    let command = CommandIdentity::from_identity(&identity, DeploymentOperation::Install)
        .map_err(|_| into_connect_error(RpcError::InvalidArgument))?;
    let owner = std::sync::Arc::clone(
        service
            .managed_recipes
            .as_ref()
            .ok_or_else(|| into_connect_error(RpcError::FailedPrecondition))?,
    );
    // Synchronous registration precedes the first cancellable transport await.
    let ticket = owner
        .install(identity, plan)
        .map_err(|_| into_connect_error(RpcError::FailedPrecondition))?;
    let budget = request::RequestBudget::from_transport(&ctx);
    let result = request::run_with_budget(&budget, ticket.wait())
        .await
        .map_err(into_connect_error)?
        .map_err(|error| into_connect_error(projection::failure(&error)))?;
    let (deployment, receipt, activations) =
        projection::result(&result, command, project, deployment, &service.receipts)
            .map_err(into_connect_error)?;
    let response = Response::ok(InstallRecipeDeploymentResponse {
        deployment: deployment.into(),
        receipt: receipt.into(),
        activations,
        ..Default::default()
    });
    drop(owner);
    response
}

pub(super) async fn resume(
    service: &ProjectRpc,
    ctx: RequestContext,
    message: ServiceRequest<'_, ResumeRecipeDeploymentRequest>,
) -> ServiceResult<ResumeRecipeDeploymentResponse> {
    let input = message.to_owned_message();
    let identity = authentication::identity(
        &ctx,
        &service.authenticator,
        RESUME,
        input.context.as_option(),
    )
    .map_err(into_connect_error)?;
    let (project, deployment) = input::resume(&input).map_err(into_connect_error)?;
    let command = CommandIdentity::from_identity(&identity, DeploymentOperation::Install)
        .map_err(|_| into_connect_error(RpcError::InvalidArgument))?;
    let owner = std::sync::Arc::clone(
        service
            .managed_recipes
            .as_ref()
            .ok_or_else(|| into_connect_error(RpcError::FailedPrecondition))?,
    );
    let ticket = owner
        .resume(identity, command, deployment, project)
        .map_err(|_| into_connect_error(RpcError::FailedPrecondition))?;
    let budget = request::RequestBudget::from_transport(&ctx);
    let result = request::run_with_budget(&budget, ticket.wait())
        .await
        .map_err(into_connect_error)?
        .map_err(|error| into_connect_error(projection::failure(&error)))?;
    let (deployment, receipt, activations) = projection::result(
        &result,
        command,
        project.as_uuid(),
        deployment,
        &service.receipts,
    )
    .map_err(into_connect_error)?;
    let response = Response::ok(ResumeRecipeDeploymentResponse {
        deployment: deployment.into(),
        receipt: receipt.into(),
        activations,
        ..Default::default()
    });
    drop(owner);
    response
}
