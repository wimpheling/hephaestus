use super::helpers::invalid;
use crate::application::commands::{CapabilitySelectionInput, InternalCommand};
use crate::rpc::{RpcError, into_connect_error, request};
use capability_domain::{CapabilityResource, CapabilitySlotKey};
use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult};
use rpc_proto::messages::hephaestus::{
    common::v1::{Diagnostic, DiagnosticCode, DiagnosticSeverity},
    instance::v1::{
        BindSecretRequest, BindSecretResponse, DeclareBrokeredHttpsRuleRequest,
        DeclareBrokeredHttpsRuleResponse, ReviseCapabilitiesRequest, ReviseCapabilitiesResponse,
    },
    secret::v1::{DeliveryMode as ProtoDeliveryMode, DeliveryPhase},
};
use secret_domain::{AgentSecretBindingId, DeliveryMode, ExecutionPhase, SecretSlotKey};
use serde_json::Value;
use uuid::Uuid;
pub(super) async fn bind_secret(
    service: &super::InstanceRpc,
    ctx: RequestContext,
    request: ServiceRequest<'_, BindSecretRequest>,
) -> ServiceResult<BindSecretResponse> {
    let request = request.to_owned_message();
    let identity =
        super::helpers::mutation(&ctx, &service.authenticator, "BindSecret", &request.context)?;
    let budget = request::RequestBudget::from_transport(&ctx);
    let mode = match request.mode.as_known() {
        Some(ProtoDeliveryMode::Raw) => DeliveryMode::Raw,
        Some(ProtoDeliveryMode::Brokered) => DeliveryMode::Brokered,
        _ => return Err(into_connect_error(RpcError::InvalidArgument)),
    };
    let phases = request
        .phases
        .into_iter()
        .map(|phase| match phase.as_known() {
            Some(DeliveryPhase::Normal) => Ok(ExecutionPhase::Normal),
            Some(DeliveryPhase::Update) => Ok(ExecutionPhase::Update),
            _ => Err(into_connect_error(RpcError::InvalidArgument)),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let attachment_ids = request
        .attachment_ids
        .iter()
        .map(|id| {
            Uuid::parse_str(&id.value).map_err(|_| into_connect_error(RpcError::InvalidArgument))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let value = super::budget::execute(
        service,
        &budget,
        &identity,
        InternalCommand::BindSecret {
            instance_id: super::helpers::parse_id(request.instance_id.as_option())?,
            expected_revision_id: super::helpers::parse_id(
                request.expected_revision_id.as_option(),
            )?,
            import_id: super::helpers::parse_id(request.import_id.as_option())?,
            slot: SecretSlotKey::parse(request.slot).map_err(invalid)?,
            mode,
            phases,
            attachment_ids,
            destinations: request.destinations,
        },
    )
    .await?;
    let receipt = super::budget::receipt(
        &budget,
        &service.receipts,
        identity.idempotency_id,
        identity.user_id,
        "agent_secret_binding",
        "agent_instance",
    )
    .await?;
    Response::ok(BindSecretResponse {
        binding_id: super::helpers::opaque(super::helpers::json_id(&value, "binding_id")?).into(),
        instance_revision_id: super::helpers::opaque(super::helpers::json_id(
            &value,
            "instance_revision_id",
        )?)
        .into(),
        receipt: receipt.into(),
        ..Default::default()
    })
}
pub(super) async fn declare_brokered_https_rule(
    service: &super::InstanceRpc,
    ctx: RequestContext,
    request: ServiceRequest<'_, DeclareBrokeredHttpsRuleRequest>,
) -> ServiceResult<DeclareBrokeredHttpsRuleResponse> {
    let request = request.to_owned_message();
    let identity = super::helpers::mutation(
        &ctx,
        &service.authenticator,
        "DeclareBrokeredHttpsRule",
        &request.context,
    )?;
    let budget = request::RequestBudget::from_transport(&ctx);
    let requested_rule_id = super::helpers::requested_rule_id(&request)?;
    let value = super::budget::execute(
        service,
        &budget,
        &identity,
        InternalCommand::DeclareBrokeredHttpsRule {
            binding_id: AgentSecretBindingId::from_uuid(super::helpers::parse_id(
                request.binding_id.as_option(),
            )?),
            destination: request.destination,
            header: request.header,
            header_prefix: request.header_prefix,
            requested_rule_id,
        },
    )
    .await?;
    let receipt = super::budget::receipt(
        &budget,
        &service.receipts,
        identity.idempotency_id,
        identity.user_id,
        "agent_secret_binding",
        "agent_instance",
    )
    .await?;
    Response::ok(DeclareBrokeredHttpsRuleResponse {
        rule_id: super::helpers::opaque(super::helpers::json_id(&value, "rule_id")?).into(),
        receipt: receipt.into(),
        ..Default::default()
    })
}
pub(super) async fn revise_capabilities(
    service: &super::InstanceRpc,
    ctx: RequestContext,
    request: ServiceRequest<'_, ReviseCapabilitiesRequest>,
) -> ServiceResult<ReviseCapabilitiesResponse> {
    let request = request.to_owned_message();
    let identity = super::helpers::mutation(
        &ctx,
        &service.authenticator,
        "ReviseCapabilities",
        &request.context,
    )?;
    let budget = request::RequestBudget::from_transport(&ctx);
    let instance_id = super::helpers::parse_id(request.instance_id.as_option())?;
    let bindings = request
        .bindings
        .into_iter()
        .map(|binding| {
            let kind = super::helpers::capability_resource_kind(&binding.resource_kind)?;
            Ok(CapabilitySelectionInput {
                slot: CapabilitySlotKey::parse(binding.slot_key).map_err(invalid)?,
                resource: CapabilityResource::new(
                    kind,
                    super::helpers::parse_id(binding.resource_id.as_option())?,
                ),
                granted_operations: binding
                    .granted_operations
                    .iter()
                    .map(|operation| super::helpers::capability_operation(operation))
                    .collect::<Result<Vec<_>, _>>()?,
            })
        })
        .collect::<Result<Vec<_>, connectrpc::ConnectError>>()?;
    let value = super::budget::execute(
        service,
        &budget,
        &identity,
        InternalCommand::ReviseCapabilities {
            instance_id,
            expected_revision_id: super::helpers::parse_id(
                request.expected_revision_id.as_option(),
            )?,
            bindings,
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
    let diagnostics = value
        .get("diagnostics")
        .and_then(Value::as_array)
        .ok_or_else(|| into_connect_error(RpcError::Internal))?
        .iter()
        .map(|diagnostic| Diagnostic {
            code: DiagnosticCode::ResourceUnavailable.into(),
            severity: DiagnosticSeverity::Error.into(),
            field: format!(
                "capability_slots.{}",
                diagnostic
                    .get("slot")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
            ),
            message: String::from("Required capability binding is missing."),
            ..Default::default()
        })
        .collect();
    Response::ok(ReviseCapabilitiesResponse {
        instance_revision_id: super::helpers::opaque(super::helpers::json_id(
            &value,
            "revision_id",
        )?)
        .into(),
        runnable: value
            .get("runnable")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        diagnostics,
        receipt: receipt.into(),
        ..Default::default()
    })
}
