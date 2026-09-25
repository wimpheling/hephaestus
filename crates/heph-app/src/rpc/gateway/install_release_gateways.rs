use super::GatewayRpc;
use crate::rpc::{RpcError, into_connect_error, mutation_receipt, request};
use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult};
use gateway_postgres::GatewayInstallError;
use release_domain::ReleaseId;
use rpc_proto::messages::hephaestus::gateway::v1::{
    InstallReleaseGatewaysRequest, InstallReleaseGatewaysResponse,
};
use std::str::FromStr;

const AUDIENCE: &str = "/hephaestus.gateway.v1.GatewayService/InstallReleaseGateways";

pub(super) async fn handle(
    service: &GatewayRpc,
    ctx: RequestContext,
    message: ServiceRequest<'_, InstallReleaseGatewaysRequest>,
) -> ServiceResult<InstallReleaseGatewaysResponse> {
    let request = message.to_owned_message();
    let identity = request::mutation_identity(
        &ctx,
        &service.authenticator,
        AUDIENCE,
        request.context.as_option(),
    )
    .map_err(into_connect_error)?;
    let release_id = request::required_id(request.release_id.as_option())
        .and_then(|value| ReleaseId::from_str(&value).map_err(|_| RpcError::InvalidArgument))
        .map_err(into_connect_error)?;
    service
        .installer_application
        .install_release(&identity, release_id)
        .await
        .map_err(|error| map_install_error(&error))
        .map_err(into_connect_error)?;
    let receipt = mutation_receipt(
        &service.receipts,
        identity.idempotency_id,
        identity.user_id,
        "gateway",
        "project",
    )
    .await?;
    Response::ok(InstallReleaseGatewaysResponse {
        receipt: receipt.into(),
        ..Default::default()
    })
}

const fn map_install_error(error: &GatewayInstallError) -> RpcError {
    match error {
        GatewayInstallError::AuthorizationDenied => RpcError::PermissionDenied,
        GatewayInstallError::InvalidManifest { .. } => RpcError::InvalidArgument,
        GatewayInstallError::Conflict => RpcError::AlreadyExists,
        GatewayInstallError::Unavailable => RpcError::FailedPrecondition,
        GatewayInstallError::Persistence(_) => RpcError::Unavailable,
    }
}
