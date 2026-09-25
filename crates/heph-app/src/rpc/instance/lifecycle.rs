use super::helpers::invalid;
use crate::application::commands::{InternalCommand, RecoveryAction as ApplicationRecoveryAction};
use crate::rpc::{RpcError, into_connect_error, request};
use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult};
use release_domain::InstanceName;
use rpc_proto::messages::hephaestus::{
    common::v1::{Operation, OperationState},
    instance::v1::{
        CreateUpdateRequest, CreateUpdateResponse, ImportAgentRequest, ImportAgentResponse,
        RecoverUpdateRequest, RecoverUpdateResponse, RecoveryAction, RecoveryDecision,
        ReviseInstanceRequest, ReviseInstanceResponse,
    },
};
use serde_json::Value;
pub(super) async fn import_agent(
    service: &super::InstanceRpc,
    ctx: RequestContext,
    request: ServiceRequest<'_, ImportAgentRequest>,
) -> ServiceResult<ImportAgentResponse> {
    let request = request.to_owned_message();
    let identity = super::helpers::mutation(
        &ctx,
        &service.authenticator,
        "ImportAgent",
        &request.context,
    )?;
    let budget = request::RequestBudget::from_transport(&ctx);
    let value = super::budget::execute(
        service,
        &budget,
        &identity,
        InternalCommand::ImportAgent {
            project_id: super::helpers::parse_id(request.project_id.as_option())?,
            release_agent_id: super::helpers::parse_id(request.release_agent_id.as_option())?,
            name: InstanceName::parse(request.name).map_err(invalid)?,
            parameters: super::helpers::parameters(request.parameters)?,
            selected_policy: super::helpers::policy(request.selected_policy.as_option())?,
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
    Response::ok(ImportAgentResponse {
        instance_id: super::helpers::opaque(super::helpers::json_id(&value, "instance_id")?).into(),
        revision_id: super::helpers::opaque(super::helpers::json_id(&value, "revision_id")?).into(),
        receipt: receipt.into(),
        ..Default::default()
    })
}
pub(super) async fn revise_instance(
    service: &super::InstanceRpc,
    ctx: RequestContext,
    request: ServiceRequest<'_, ReviseInstanceRequest>,
) -> ServiceResult<ReviseInstanceResponse> {
    let request = request.to_owned_message();
    let identity = super::helpers::mutation(
        &ctx,
        &service.authenticator,
        "ReviseInstance",
        &request.context,
    )?;
    let budget = request::RequestBudget::from_transport(&ctx);
    let instance_id = super::helpers::parse_id(request.instance_id.as_option())?;
    let value = super::budget::execute(
        service,
        &budget,
        &identity,
        InternalCommand::ReviseInstance {
            instance_id,
            expected_revision_id: super::helpers::parse_id(
                request.expected_revision_id.as_option(),
            )?,
            parameters: super::helpers::parameters(request.parameters)?,
            selected_policy: super::helpers::policy(request.selected_policy.as_option())?,
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
    Response::ok(ReviseInstanceResponse {
        instance_id: super::helpers::opaque(instance_id.to_string()).into(),
        revision_id: super::helpers::opaque(super::helpers::json_id(&value, "revision_id")?).into(),
        receipt: receipt.into(),
        ..Default::default()
    })
}
pub(super) async fn create_update(
    service: &super::InstanceRpc,
    ctx: RequestContext,
    request: ServiceRequest<'_, CreateUpdateRequest>,
) -> ServiceResult<CreateUpdateResponse> {
    let request = request.to_owned_message();
    let identity = super::helpers::mutation(
        &ctx,
        &service.authenticator,
        "CreateUpdate",
        &request.context,
    )?;
    let budget = request::RequestBudget::from_transport(&ctx);
    let value = super::budget::execute(
        service,
        &budget,
        &identity,
        InternalCommand::CreateUpdate {
            instance_id: super::helpers::parse_id(request.instance_id.as_option())?,
            expected_revision_id: super::helpers::parse_id(
                request.expected_revision_id.as_option(),
            )?,
            candidate_release_agent_id: super::helpers::parse_id(
                request.candidate_release_agent_id.as_option(),
            )?,
            parameters: super::helpers::parameters(request.parameters)?,
            brokered_rule_copies: super::helpers::brokered_rule_copies(
                request.brokered_rule_copies,
            )?,
            selected_policy: super::helpers::policy(request.selected_policy.as_option())?,
        },
    )
    .await?;
    let hook_run_id = value
        .get("hook_run_id")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let receipt = super::budget::receipt(
        &budget,
        &service.receipts,
        identity.idempotency_id,
        identity.user_id,
        "agent_instance",
        "agent_instance",
    )
    .await?;
    Response::ok(CreateUpdateResponse {
        update_id: super::helpers::opaque(super::helpers::json_id(&value, "update_id")?).into(),
        candidate_revision_id: super::helpers::opaque(super::helpers::json_id(
            &value,
            "candidate_revision_id",
        )?)
        .into(),
        hook_run_id: hook_run_id
            .as_ref()
            .map(|id| super::helpers::opaque(id.clone()))
            .into(),
        operation: hook_run_id
            .map(|id| {
                Operation {
                    id: super::helpers::opaque(id).into(),
                    state: OperationState::Queued.into(),
                    ..Default::default()
                }
                .into()
            })
            .unwrap_or_default(),
        receipt: receipt.into(),
        ..Default::default()
    })
}
pub(super) async fn recover_update(
    service: &super::InstanceRpc,
    ctx: RequestContext,
    request: ServiceRequest<'_, RecoverUpdateRequest>,
) -> ServiceResult<RecoverUpdateResponse> {
    let request = request.to_owned_message();
    let identity = super::helpers::mutation(
        &ctx,
        &service.authenticator,
        "RecoverUpdate",
        &request.context,
    )?;
    let budget = request::RequestBudget::from_transport(&ctx);
    let update_id = super::helpers::parse_id(request.update_id.as_option())?;
    let (action, decision) = match request.action.as_known() {
        Some(RecoveryAction::Retry) => (
            ApplicationRecoveryAction::Retry,
            RecoveryDecision::RetryQueued,
        ),
        Some(RecoveryAction::Reject) => (
            ApplicationRecoveryAction::Reject,
            RecoveryDecision::Rejected,
        ),
        Some(RecoveryAction::Resume) => {
            (ApplicationRecoveryAction::Resume, RecoveryDecision::Resumed)
        }
        _ => return Err(into_connect_error(RpcError::InvalidArgument)),
    };
    super::budget::execute(
        service,
        &budget,
        &identity,
        InternalCommand::RecoverUpdate { update_id, action },
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
    Response::ok(RecoverUpdateResponse {
        update_id: super::helpers::opaque(update_id.to_string()).into(),
        decision: decision.into(),
        receipt: receipt.into(),
        ..Default::default()
    })
}
