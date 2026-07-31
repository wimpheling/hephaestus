//! Agent-instance RPC adapters.

mod get_instance;

use super::{
    MediatorAuthenticator, MutationReceipts, RpcError, into_connect_error, mutation_receipt,
    request,
};
use crate::application::commands::{
    InternalCommand, InternalCommandState, RecoveryAction as ApplicationRecoveryAction, dispatch,
};
use crate::application::instance::InstanceApplication;
use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult};
use control_plane_postgres::ControlPlanePool as PgPool;
use release_domain::{
    InstanceName, NetworkAccess, ParameterName, ParameterValue, RefSelector, RuntimePolicy,
    TriggerPolicy,
};
use rpc_proto::{
    connect::hephaestus::instance::v1::AgentInstanceService,
    messages::hephaestus::{
        common::v1::{
            NetworkPolicy, OpaqueId, Operation, OperationState,
            ParameterValue as ProtoParameterValue, RuntimePolicy as ProtoRuntimePolicy,
            parameter_value,
        },
        instance::v1::{
            BindSecretRequest, BindSecretResponse, CreateAttachmentRequest,
            CreateAttachmentResponse, CreateUpdateRequest, CreateUpdateResponse,
            GetInstanceRequest, GetInstanceResponse, ImportAgentRequest, ImportAgentResponse,
            RecoverUpdateRequest, RecoverUpdateResponse, RecoveryAction, RecoveryDecision,
            RemovalState, RemoveAttachmentRequest, RemoveAttachmentResponse, ReviseInstanceRequest,
            ReviseInstanceResponse, SetAttachmentEnabledRequest, SetAttachmentEnabledResponse,
            TriggerPolicy as ProtoTriggerPolicy, ref_selector,
        },
        secret::v1::{DeliveryMode as ProtoDeliveryMode, DeliveryPhase},
    },
};
use secret_domain::{DeliveryMode, ExecutionPhase, SecretSlotKey};
use serde_json::Value;
use std::{collections::BTreeMap, str::FromStr};
use uuid::Uuid;

/// Generated instance service backed by the existing release application.
pub struct InstanceRpc {
    application: InstanceApplication,
    commands: InternalCommandState,
    authenticator: MediatorAuthenticator,
    receipts: MutationReceipts,
}

impl InstanceRpc {
    /// Creates an instance service using the shared application command state.
    pub const fn new(
        pool: PgPool,
        commands: InternalCommandState,
        authenticator: MediatorAuthenticator,
        receipts: MutationReceipts,
    ) -> Self {
        Self {
            application: InstanceApplication::new(pool),
            commands,
            authenticator,
            receipts,
        }
    }

    async fn execute(
        &self,
        identity: &identity_domain::AuthenticatedIdentity,
        command: InternalCommand,
    ) -> Result<Value, RpcError> {
        dispatch(&self.commands, identity, command)
            .await
            .map_err(|error| {
                tracing::warn!(
                    actor_id = %identity.user_id,
                    request_id = %identity.request_id,
                    %error,
                    "RPC command rejected"
                );
                RpcError::FailedPrecondition
            })
    }
}

// Generated traits hide an Encodable response; concrete message bodies keep
// each adapter readable while refining only this implementation's opaque type.
#[allow(refining_impl_trait)]
impl AgentInstanceService for InstanceRpc {
    async fn get_instance(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetInstanceRequest>,
    ) -> ServiceResult<GetInstanceResponse> {
        get_instance::handle(self, ctx, request).await
    }

    async fn import_agent(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ImportAgentRequest>,
    ) -> ServiceResult<ImportAgentResponse> {
        let request = request.to_owned_message();
        let identity = mutation(&ctx, &self.authenticator, "ImportAgent", &request.context)?;
        let value = self
            .execute(
                &identity,
                InternalCommand::ImportAgent {
                    project_id: parse_id(request.project_id.as_option())?,
                    release_agent_id: parse_id(request.release_agent_id.as_option())?,
                    name: InstanceName::parse(request.name).map_err(invalid)?,
                    parameters: parameters(request.parameters)?,
                    selected_policy: policy(request.selected_policy.as_option())?,
                },
            )
            .await
            .map_err(into_connect_error)?;
        let receipt = mutation_receipt(
            &self.receipts,
            identity.idempotency_id,
            identity.user_id,
            "agent_instance",
            "agent_instance",
        )
        .await?;
        Response::ok(ImportAgentResponse {
            instance_id: opaque(json_id(&value, "instance_id")?).into(),
            revision_id: opaque(json_id(&value, "revision_id")?).into(),
            receipt: receipt.into(),
            ..Default::default()
        })
    }

    async fn create_attachment(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CreateAttachmentRequest>,
    ) -> ServiceResult<CreateAttachmentResponse> {
        let request = request.to_owned_message();
        let identity = mutation(
            &ctx,
            &self.authenticator,
            "CreateAttachment",
            &request.context,
        )?;
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
        let value = self
            .execute(
                &identity,
                InternalCommand::CreateAttachment {
                    instance_id: parse_id(request.instance_id.as_option())?,
                    repository_id: parse_id(request.repository_id.as_option())?,
                    ref_selector: selector,
                    trigger_policy,
                },
            )
            .await
            .map_err(into_connect_error)?;
        let receipt = mutation_receipt(
            &self.receipts,
            identity.idempotency_id,
            identity.user_id,
            "agent_instance",
            "agent_instance",
        )
        .await?;
        Response::ok(CreateAttachmentResponse {
            attachment_id: opaque(json_id(&value, "attachment_id")?).into(),
            receipt: receipt.into(),
            ..Default::default()
        })
    }

    async fn set_attachment_enabled(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SetAttachmentEnabledRequest>,
    ) -> ServiceResult<SetAttachmentEnabledResponse> {
        let request = request.to_owned_message();
        let identity = mutation(
            &ctx,
            &self.authenticator,
            "SetAttachmentEnabled",
            &request.context,
        )?;
        let attachment_id = parse_id(request.attachment_id.as_option())?;
        self.execute(
            &identity,
            InternalCommand::SetAttachmentEnabled {
                attachment_id,
                enabled: request.enabled,
            },
        )
        .await
        .map_err(into_connect_error)?;
        let receipt = mutation_receipt(
            &self.receipts,
            identity.idempotency_id,
            identity.user_id,
            "agent_instance",
            "agent_instance",
        )
        .await?;
        Response::ok(SetAttachmentEnabledResponse {
            attachment_id: opaque(attachment_id.to_string()).into(),
            enabled: request.enabled,
            receipt: receipt.into(),
            ..Default::default()
        })
    }

    async fn remove_attachment(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RemoveAttachmentRequest>,
    ) -> ServiceResult<RemoveAttachmentResponse> {
        let request = request.to_owned_message();
        let identity = mutation(
            &ctx,
            &self.authenticator,
            "RemoveAttachment",
            &request.context,
        )?;
        let attachment_id = parse_id(request.attachment_id.as_option())?;
        self.execute(
            &identity,
            InternalCommand::RemoveAttachment { attachment_id },
        )
        .await
        .map_err(into_connect_error)?;
        let receipt = mutation_receipt(
            &self.receipts,
            identity.idempotency_id,
            identity.user_id,
            "agent_instance",
            "agent_instance",
        )
        .await?;
        Response::ok(RemoveAttachmentResponse {
            attachment_id: opaque(attachment_id.to_string()).into(),
            state: RemovalState::Removed.into(),
            receipt: receipt.into(),
            ..Default::default()
        })
    }

    async fn revise_instance(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ReviseInstanceRequest>,
    ) -> ServiceResult<ReviseInstanceResponse> {
        let request = request.to_owned_message();
        let identity = mutation(
            &ctx,
            &self.authenticator,
            "ReviseInstance",
            &request.context,
        )?;
        let instance_id = parse_id(request.instance_id.as_option())?;
        let value = self
            .execute(
                &identity,
                InternalCommand::ReviseInstance {
                    instance_id,
                    expected_revision_id: parse_id(request.expected_revision_id.as_option())?,
                    parameters: parameters(request.parameters)?,
                    selected_policy: policy(request.selected_policy.as_option())?,
                },
            )
            .await
            .map_err(into_connect_error)?;
        let receipt = mutation_receipt(
            &self.receipts,
            identity.idempotency_id,
            identity.user_id,
            "agent_instance",
            "agent_instance",
        )
        .await?;
        Response::ok(ReviseInstanceResponse {
            instance_id: opaque(instance_id.to_string()).into(),
            revision_id: opaque(json_id(&value, "revision_id")?).into(),
            receipt: receipt.into(),
            ..Default::default()
        })
    }

    async fn create_update(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CreateUpdateRequest>,
    ) -> ServiceResult<CreateUpdateResponse> {
        let request = request.to_owned_message();
        let identity = mutation(&ctx, &self.authenticator, "CreateUpdate", &request.context)?;
        let value = self
            .execute(
                &identity,
                InternalCommand::CreateUpdate {
                    instance_id: parse_id(request.instance_id.as_option())?,
                    expected_revision_id: parse_id(request.expected_revision_id.as_option())?,
                    candidate_release_agent_id: parse_id(
                        request.candidate_release_agent_id.as_option(),
                    )?,
                    parameters: parameters(request.parameters)?,
                    selected_policy: policy(request.selected_policy.as_option())?,
                },
            )
            .await
            .map_err(into_connect_error)?;
        let hook_run_id = json_id(&value, "hook_run_id")?;
        let receipt = mutation_receipt(
            &self.receipts,
            identity.idempotency_id,
            identity.user_id,
            "agent_instance",
            "agent_instance",
        )
        .await?;
        Response::ok(CreateUpdateResponse {
            update_id: opaque(json_id(&value, "update_id")?).into(),
            candidate_revision_id: opaque(json_id(&value, "candidate_revision_id")?).into(),
            hook_run_id: opaque(hook_run_id.clone()).into(),
            operation: Operation {
                id: opaque(hook_run_id).into(),
                state: OperationState::Queued.into(),
                ..Default::default()
            }
            .into(),
            receipt: receipt.into(),
            ..Default::default()
        })
    }

    async fn recover_update(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RecoverUpdateRequest>,
    ) -> ServiceResult<RecoverUpdateResponse> {
        let request = request.to_owned_message();
        let identity = mutation(&ctx, &self.authenticator, "RecoverUpdate", &request.context)?;
        let update_id = parse_id(request.update_id.as_option())?;
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
        self.execute(
            &identity,
            InternalCommand::RecoverUpdate { update_id, action },
        )
        .await
        .map_err(into_connect_error)?;
        let receipt = mutation_receipt(
            &self.receipts,
            identity.idempotency_id,
            identity.user_id,
            "agent_instance",
            "agent_instance",
        )
        .await?;
        Response::ok(RecoverUpdateResponse {
            update_id: opaque(update_id.to_string()).into(),
            decision: decision.into(),
            receipt: receipt.into(),
            ..Default::default()
        })
    }

    async fn bind_secret(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, BindSecretRequest>,
    ) -> ServiceResult<BindSecretResponse> {
        let request = request.to_owned_message();
        let identity = mutation(&ctx, &self.authenticator, "BindSecret", &request.context)?;
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
                Uuid::parse_str(&id.value)
                    .map_err(|_| into_connect_error(RpcError::InvalidArgument))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let value = self
            .execute(
                &identity,
                InternalCommand::BindSecret {
                    instance_id: parse_id(request.instance_id.as_option())?,
                    expected_revision_id: parse_id(request.expected_revision_id.as_option())?,
                    import_id: parse_id(request.import_id.as_option())?,
                    slot: SecretSlotKey::parse(request.slot).map_err(invalid)?,
                    mode,
                    phases,
                    attachment_ids,
                    destinations: request.destinations,
                },
            )
            .await
            .map_err(into_connect_error)?;
        let receipt = mutation_receipt(
            &self.receipts,
            identity.idempotency_id,
            identity.user_id,
            "agent_secret_binding",
            "agent_instance",
        )
        .await?;
        Response::ok(BindSecretResponse {
            binding_id: opaque(json_id(&value, "binding_id")?).into(),
            instance_revision_id: opaque(json_id(&value, "instance_revision_id")?).into(),
            receipt: receipt.into(),
            ..Default::default()
        })
    }
}

fn mutation(
    ctx: &RequestContext,
    authenticator: &MediatorAuthenticator,
    method: &str,
    context: &buffa::MessageField<rpc_proto::messages::hephaestus::common::v1::RequestContext>,
) -> Result<identity_domain::AuthenticatedIdentity, connectrpc::ConnectError> {
    request::mutation_identity(
        ctx,
        authenticator,
        &format!("/hephaestus.instance.v1.AgentInstanceService/{method}"),
        context.as_option(),
    )
    .map_err(into_connect_error)
}

fn parse_id<T: FromStr>(value: Option<&OpaqueId>) -> Result<T, connectrpc::ConnectError> {
    request::required_id(value)
        .map_err(into_connect_error)?
        .parse()
        .map_err(|_| into_connect_error(RpcError::InvalidArgument))
}

fn policy(value: Option<&ProtoRuntimePolicy>) -> Result<RuntimePolicy, connectrpc::ConnectError> {
    let value = value.ok_or_else(|| into_connect_error(RpcError::InvalidArgument))?;
    let vcpus =
        u8::try_from(value.vcpus).map_err(|_| into_connect_error(RpcError::InvalidArgument))?;
    if vcpus == 0 || value.memory_mib == 0 {
        return Err(into_connect_error(RpcError::InvalidArgument));
    }
    let network = match value.network.as_known() {
        Some(NetworkPolicy::Disabled) => NetworkAccess::Disabled,
        Some(NetworkPolicy::BrokerOnly) => NetworkAccess::BrokerOnly,
        _ => return Err(into_connect_error(RpcError::InvalidArgument)),
    };
    Ok(RuntimePolicy {
        vcpus,
        memory_mib: value.memory_mib,
        network,
    })
}

fn parameters(
    values: Vec<ProtoParameterValue>,
) -> Result<BTreeMap<ParameterName, ParameterValue>, connectrpc::ConnectError> {
    values
        .into_iter()
        .map(|value| {
            let name = ParameterName::parse(value.name).map_err(invalid)?;
            let value = match value.value {
                Some(parameter_value::Value::StringValue(value)) => ParameterValue::String(value),
                Some(parameter_value::Value::IntegerValue(value)) => ParameterValue::Integer(value),
                Some(parameter_value::Value::BooleanValue(value)) => ParameterValue::Boolean(value),
                None => return Err(into_connect_error(RpcError::InvalidArgument)),
            };
            Ok((name, value))
        })
        .collect()
}

fn json_id(value: &Value, field: &str) -> Result<String, connectrpc::ConnectError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| into_connect_error(RpcError::Internal))
}

fn opaque(value: String) -> OpaqueId {
    OpaqueId {
        value,
        ..Default::default()
    }
}

fn invalid<T>(_error: T) -> connectrpc::ConnectError {
    into_connect_error(RpcError::InvalidArgument)
}
