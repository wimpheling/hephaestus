use super::model::{action_error, opaque, operation_state, timestamp};
use crate::rpc::{RpcError, into_connect_error, mutation_receipt, request};
use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult};
use rpc_proto::messages::hephaestus::{
    build::v1::{RetryBuildRequest, RetryBuildResponse},
    common::v1::Operation,
};
use uuid::Uuid;

const AUDIENCE: &str = "/hephaestus.build.v1.BuildService/RetryBuild";

pub(super) async fn handle(
    service: &super::BuildRpc,
    ctx: RequestContext,
    request_message: ServiceRequest<'_, RetryBuildRequest>,
) -> ServiceResult<RetryBuildResponse> {
    let request = request_message.to_owned_message();
    let identity = request::mutation_identity(
        &ctx,
        &service.authenticator,
        AUDIENCE,
        request.context.as_option(),
    )
    .map_err(into_connect_error)?;
    let id = request::required_id(request.build_id.as_option())
        .and_then(|value| Uuid::parse_str(&value).map_err(|_| RpcError::InvalidArgument))
        .map_err(into_connect_error)?;
    let result = service
        .application
        .retry_build(&identity, id)
        .await
        .map_err(action_error)
        .map_err(into_connect_error)?;
    let receipt = mutation_receipt(
        &service.receipts,
        identity.idempotency_id,
        identity.user_id,
        "build",
        "repository",
    )
    .await?;
    Response::ok(RetryBuildResponse {
        build_id: opaque(result.id).into(),
        operation: Operation {
            id: opaque(result.id).into(),
            state: operation_state(result.state).into(),
            created_at: timestamp(result.created_at).into(),
            updated_at: timestamp(result.updated_at).into(),
            ..Default::default()
        }
        .into(),
        receipt: receipt.into(),
        ..Default::default()
    })
}
