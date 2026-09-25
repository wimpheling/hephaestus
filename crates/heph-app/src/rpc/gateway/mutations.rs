use super::GatewayRpc;
use super::auth::{id, mutation, receipt};
use super::conversions::{lifecycle, mailbox_binding, map_error, summary};
use crate::rpc::{RpcError, into_connect_error, request};
use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult};
use rpc_proto::messages::hephaestus::gateway::v1::{
    CreateMailboxBindingRequest, CreateMailboxBindingResponse, RevokeMailboxBindingGrantRequest,
    RevokeMailboxBindingGrantResponse, SetGatewayLifecycleRequest, SetGatewayLifecycleResponse,
};

pub(super) async fn set_gateway_lifecycle(
    service: &GatewayRpc,
    ctx: RequestContext,
    message: ServiceRequest<'_, SetGatewayLifecycleRequest>,
) -> ServiceResult<SetGatewayLifecycleResponse> {
    let budget = request::RequestBudget::from_transport(&ctx);
    let request = message.to_owned_message();
    let identity = request::mutation_identity(
        &ctx,
        &service.authenticator,
        "/hephaestus.gateway.v1.GatewayService/SetGatewayLifecycle",
        request.context.as_option(),
    )
    .map_err(into_connect_error)?;
    let gateway_id = id(request.gateway_id.as_option())?;
    let changed = request::run_with_budget(
        &budget,
        service.application.transition(
            &identity,
            gateway_id,
            lifecycle(
                request
                    .expected
                    .as_known()
                    .ok_or_else(|| into_connect_error(RpcError::InvalidArgument))?,
            )?,
            lifecycle(
                request
                    .next
                    .as_known()
                    .ok_or_else(|| into_connect_error(RpcError::InvalidArgument))?,
            )?,
        ),
    )
    .await
    .map_err(into_connect_error)?
    .map_err(|error| map_error(&error))
    .map_err(into_connect_error)?;
    if !changed {
        return Err(into_connect_error(RpcError::FailedPrecondition));
    }
    let (gateway, _) =
        request::run_with_budget(&budget, service.application.get(&identity, gateway_id))
            .await
            .map_err(into_connect_error)?
            .map_err(|error| map_error(&error))
            .map_err(into_connect_error)?;
    let receipt = request::run_with_budget(&budget, receipt(&service.receipts, &identity))
        .await
        .map_err(into_connect_error)??;
    Response::ok(SetGatewayLifecycleResponse {
        gateway: summary(gateway).into(),
        receipt: receipt.into(),
        ..Default::default()
    })
}

pub(super) async fn create_mailbox_binding(
    service: &GatewayRpc,
    ctx: RequestContext,
    message: ServiceRequest<'_, CreateMailboxBindingRequest>,
) -> ServiceResult<CreateMailboxBindingResponse> {
    let budget = request::RequestBudget::from_transport(&ctx);
    let request = message.to_owned_message();
    let identity = mutation(
        &ctx,
        &service.authenticator,
        "CreateMailboxBinding",
        request.context.as_option(),
    )?;
    let binding = request::run_with_budget(
        &budget,
        service.application.create_mailbox_binding(
            &identity,
            id(request.gateway_revision_id.as_option())?,
            &request.slot_key,
            id(request.mailbox_id.as_option())?,
            &request.producer_id,
        ),
    )
    .await
    .map_err(into_connect_error)?
    .map_err(|error| map_error(&error))
    .map_err(into_connect_error)?;
    let receipt = request::run_with_budget(&budget, receipt(&service.receipts, &identity))
        .await
        .map_err(into_connect_error)??;
    Response::ok(CreateMailboxBindingResponse {
        binding: mailbox_binding(binding).into(),
        receipt: receipt.into(),
        ..Default::default()
    })
}

pub(super) async fn revoke_mailbox_binding_grant(
    service: &GatewayRpc,
    ctx: RequestContext,
    message: ServiceRequest<'_, RevokeMailboxBindingGrantRequest>,
) -> ServiceResult<RevokeMailboxBindingGrantResponse> {
    let budget = request::RequestBudget::from_transport(&ctx);
    let request = message.to_owned_message();
    let identity = mutation(
        &ctx,
        &service.authenticator,
        "RevokeMailboxBindingGrant",
        request.context.as_option(),
    )?;
    let binding = request::run_with_budget(
        &budget,
        service
            .application
            .revoke_mailbox_binding_grant(&identity, id(request.binding_id.as_option())?),
    )
    .await
    .map_err(into_connect_error)?
    .map_err(|error| map_error(&error))
    .map_err(into_connect_error)?;
    let receipt = request::run_with_budget(&budget, receipt(&service.receipts, &identity))
        .await
        .map_err(into_connect_error)??;
    Response::ok(RevokeMailboxBindingGrantResponse {
        binding: mailbox_binding(binding).into(),
        receipt: receipt.into(),
        ..Default::default()
    })
}
