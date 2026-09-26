use super::helpers::invalid;
use crate::application::commands::InternalCommand;
use crate::rpc::{RpcError, into_connect_error, request};
use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult};
use release_domain::{RefSelector, TriggerPolicy};
use rpc_proto::messages::hephaestus::instance::v1::{
    CreateAttachmentRequest, CreateAttachmentResponse, RemovalState, RemoveAttachmentRequest,
    RemoveAttachmentResponse, SetAttachmentEnabledRequest, SetAttachmentEnabledResponse,
    TriggerPolicy as ProtoTriggerPolicy, ref_selector,
};
pub(super) async fn create_attachment(
    service: &super::InstanceRpc,
    ctx: RequestContext,
    request: ServiceRequest<'_, CreateAttachmentRequest>,
) -> ServiceResult<CreateAttachmentResponse> {
    let request = request.to_owned_message();
    let identity = super::helpers::mutation(
        &ctx,
        &service.authenticator,
        "CreateAttachment",
        &request.context,
    )?;
    let budget = request::RequestBudget::from_transport(&ctx);
    let selector = request
        .ref_selector
        .as_option()
        .and_then(|value| value.selector.as_ref())
        .ok_or_else(|| into_connect_error(RpcError::InvalidArgument))?;
    let selector = match selector {
        ref_selector::Selector::Exact(value) => RefSelector::parse(value.clone()),
        ref_selector::Selector::Prefix(value) => RefSelector::parse(format!("{value}/*")),
    }
    .map_err(invalid)?;
    let trigger_policy = match request.trigger_policy.as_known() {
        Some(ProtoTriggerPolicy::Manual) => TriggerPolicy::Manual,
        Some(ProtoTriggerPolicy::Push) => TriggerPolicy::Push,
        Some(ProtoTriggerPolicy::PushAndManual) => TriggerPolicy::PushAndManual,
        _ => return Err(into_connect_error(RpcError::InvalidArgument)),
    };
    let value = super::budget::execute(
        service,
        &budget,
        &identity,
        InternalCommand::CreateAttachment {
            instance_id: super::helpers::parse_id(request.instance_id.as_option())?,
            repository_id: super::helpers::parse_id(request.repository_id.as_option())?,
            ref_selector: selector,
            trigger_policy,
        },
    )
    .await?;
    let receipt = super::budget::receipt(
        &budget,
        &service.receipts,
        identity.idempotency_id,
        identity.user_id,
        "agent_instance",
        "agent_instance",
    )
    .await?;
    Response::ok(CreateAttachmentResponse {
        attachment_id: super::helpers::opaque(super::helpers::json_id(&value, "attachment_id")?)
            .into(),
        receipt: receipt.into(),
        ..Default::default()
    })
}
pub(super) async fn set_attachment_enabled(
    service: &super::InstanceRpc,
    ctx: RequestContext,
    request: ServiceRequest<'_, SetAttachmentEnabledRequest>,
) -> ServiceResult<SetAttachmentEnabledResponse> {
    let request = request.to_owned_message();
    let identity = super::helpers::mutation(
        &ctx,
        &service.authenticator,
        "SetAttachmentEnabled",
        &request.context,
    )?;
    let budget = request::RequestBudget::from_transport(&ctx);
    let attachment_id = super::helpers::parse_id(request.attachment_id.as_option())?;
    super::budget::execute(
        service,
        &budget,
        &identity,
        InternalCommand::SetAttachmentEnabled {
            attachment_id,
            enabled: request.enabled,
        },
    )
    .await?;
    let receipt = super::budget::receipt(
        &budget,
        &service.receipts,
        identity.idempotency_id,
        identity.user_id,
        "agent_instance",
        "agent_instance",
    )
    .await?;
    Response::ok(SetAttachmentEnabledResponse {
        attachment_id: super::helpers::opaque(attachment_id.to_string()).into(),
        enabled: request.enabled,
        receipt: receipt.into(),
        ..Default::default()
    })
}
pub(super) async fn remove_attachment(
    service: &super::InstanceRpc,
    ctx: RequestContext,
    request: ServiceRequest<'_, RemoveAttachmentRequest>,
) -> ServiceResult<RemoveAttachmentResponse> {
    let request = request.to_owned_message();
    let identity = super::helpers::mutation(
        &ctx,
        &service.authenticator,
        "RemoveAttachment",
        &request.context,
    )?;
    let budget = request::RequestBudget::from_transport(&ctx);
    let attachment_id = super::helpers::parse_id(request.attachment_id.as_option())?;
    super::budget::execute(
        service,
        &budget,
        &identity,
        InternalCommand::RemoveAttachment { attachment_id },
    )
    .await?;
    let receipt = super::budget::receipt(
        &budget,
        &service.receipts,
        identity.idempotency_id,
        identity.user_id,
        "agent_instance",
        "agent_instance",
    )
    .await?;
    Response::ok(RemoveAttachmentResponse {
        attachment_id: super::helpers::opaque(attachment_id.to_string()).into(),
        state: RemovalState::Removed.into(),
        receipt: receipt.into(),
        ..Default::default()
    })
}
