use super::InstanceRpc;
use crate::{
    application::instance::{InstanceQueryError, InstanceSnapshot},
    rpc::{RpcError, into_connect_error, request},
};
use buffa::Message as _;
use buffa_types::google::protobuf::Timestamp;
use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult};
use rpc_proto::messages::hephaestus::{
    common::v1::{
        Diagnostic, DiagnosticCode, DiagnosticSeverity, EnumParameterConstraints,
        IntegerParameterConstraints, NetworkPolicy, OpaqueId, OperationState, ParameterDeclaration,
        ParameterDefault, ParameterType, ParameterValue, RuntimeContract, RuntimePolicy,
        SecretSlotDeclaration, SecretSlotDeliveryMode, SecretSlotPhase, StringParameterConstraints,
        UpdateHook, parameter_default, parameter_type, parameter_value,
    },
    instance::v1::{
        AgentInstance, AgentUpdate, Attachment, GetInstanceRequest, GetInstanceResponse, RecentRun,
        RecoveryDecision, RefSelector, RepositoryOption, SecretImport, TriggerPolicy,
        UpdateCandidate, UpdateEvent, ref_selector, update_event,
    },
    secret::v1::{
        AuthorityState, DeliveryMode, DeliveryPhase, SecretPolicy, SecretState, SecretTarget,
        secret_target,
    },
};
use serde_json::Value;
use time::OffsetDateTime;
use uuid::Uuid;

const MAX_RESPONSE_BYTES: u32 = 4 * 1_048_576;
const MAX_HOOK_EVENTS: usize = 500;

pub(super) async fn handle(
    service: &InstanceRpc,
    ctx: RequestContext,
    message: ServiceRequest<'_, GetInstanceRequest>,
) -> ServiceResult<GetInstanceResponse> {
    let identity = request::query_identity(
        &ctx,
        &service.authenticator,
        "/hephaestus.instance.v1.AgentInstanceService/GetInstance",
    )
    .map_err(into_connect_error)?;
    let instance_id = request::required_id(message.to_owned_message().instance_id.as_option())
        .and_then(|value| value.parse().map_err(|_| RpcError::InvalidArgument))
        .map_err(into_connect_error)?;
    let snapshot = service
        .application
        .get(&identity, instance_id)
        .await
        .map_err(map_error)
        .map_err(into_connect_error)?;
    let response = GetInstanceResponse {
        instance: project(snapshot).map_err(into_connect_error)?.into(),
        ..Default::default()
    };
    ensure_response_bound(&response).map_err(into_connect_error)?;
    Response::ok(response)
}

fn ensure_response_bound(response: &GetInstanceResponse) -> Result<(), RpcError> {
    if response.encoded_len() > MAX_RESPONSE_BYTES {
        Err(RpcError::ResourceExhausted)
    } else {
        Ok(())
    }
}

// The generated response intentionally projects the complete bounded instance snapshot.
#[allow(clippy::too_many_lines)]
fn project(snapshot: InstanceSnapshot) -> Result<AgentInstance, RpcError> {
    let revisions = snapshot
        .revisions
        .into_iter()
        .map(|row| {
            Ok(
                rpc_proto::messages::hephaestus::instance::v1::InstanceRevision {
                    id: opaque(row.id).into(),
                    parameters: parameters(&row.parameters)?,
                    parameter_hash: encode_hex(&row.parameter_hash),
                    resource_selection: policy(&row.resource_selection)?.into(),
                    network_restriction: network_restriction(&row.network_restriction)?.into(),
                    effective_runtime_policy: policy(&row.effective_runtime_policy)?.into(),
                    platform_policy_version: row.platform_policy_version.clone(),
                    runnable: row.runnable,
                    diagnostics: diagnostics(&row.diagnostics)?,
                    created_at: timestamp(row.created_at).into(),
                    release_agent_id: opaque(row.release_agent_id).into(),
                    parameter_schema: parameter_schema(&row.parameter_schema)?,
                    secret_slot_schema: secret_slots(&row.secret_slot_schema)?,
                    runtime_contract: contract(
                        &row.runtime_contract,
                        &row.platform_policy_version,
                    )?
                    .into(),
                    update_hook: update_hook(row.update_hook.as_ref()).into(),
                    release_id: opaque(row.release_id).into(),
                    release_version: row.release_version,
                    release_state: row.release_state,
                    release_agent_name: row.release_agent_name,
                    ..Default::default()
                },
            )
        })
        .collect::<Result<Vec<_>, RpcError>>()?;
    let attachments = snapshot
        .attachments
        .into_iter()
        .map(|row| {
            Ok(Attachment {
                id: opaque(row.id).into(),
                ref_selector: selector(&row.ref_selector).into(),
                trigger_policy: trigger(&row.trigger_policy)?.into(),
                enabled: row.enabled,
                removed_at: row.removed_at.map(timestamp).into(),
                repository_id: opaque(row.repository_id).into(),
                repository_name: row.repository_name,
                can_manage: row.can_manage,
                ..Default::default()
            })
        })
        .collect::<Result<Vec<_>, RpcError>>()?;
    let updates = snapshot
        .updates
        .into_iter()
        .map(|row| {
            Ok(AgentUpdate {
                id: opaque(row.id).into(),
                expected_current_revision_id: opaque(row.expected_current_revision_id).into(),
                candidate_revision_id: opaque(row.candidate_revision_id).into(),
                state: row.state,
                hook_run_id: row.hook_run_id.map(opaque).into(),
                hook_exit_code: row.hook_exit_code,
                hook_exit_signal: row.hook_exit_signal,
                diagnostics: diagnostics(&row.diagnostics)?,
                final_decision: recovery(row.final_decision.as_deref()).into(),
                created_at: timestamp(row.created_at).into(),
                updated_at: timestamp(row.updated_at).into(),
                hook_events: events(&row.hook_events)?,
                ..Default::default()
            })
        })
        .collect::<Result<Vec<_>, RpcError>>()?;
    let repositories = snapshot
        .repositories
        .into_iter()
        .map(|row| RepositoryOption {
            id: opaque(row.id).into(),
            name: row.name,
            default_branch: row.default_branch,
            ..Default::default()
        })
        .collect();
    let secret_imports = snapshot
        .imports
        .into_iter()
        .map(|row| {
            Ok(SecretImport {
                id: opaque(row.id).into(),
                r#alias: row.alias,
                target: target(&row.target_kind, row.target_id)?.into(),
                state: authority(&row.status)?.into(),
                secret_name: row.secret_name,
                secret_state: secret_state(&row.secret_status)?.into(),
                policy: SecretPolicy {
                    delivery_modes: row
                        .delivery_modes
                        .iter()
                        .map(|value| delivery(value))
                        .collect::<Result<Vec<_>, _>>()?,
                    phases: row
                        .phases
                        .iter()
                        .map(|value| phase(value))
                        .collect::<Result<Vec<_>, _>>()?,
                    destinations: row.destinations,
                    ..Default::default()
                }
                .into(),
                expires_at: row.expires_at.map(timestamp).into(),
                ..Default::default()
            })
        })
        .collect::<Result<Vec<_>, RpcError>>()?;
    let update_candidates = snapshot
        .candidates
        .into_iter()
        .map(|row| {
            Ok(UpdateCandidate {
                id: opaque(row.id).into(),
                display_name: row.display_name,
                parameter_schema: parameter_schema(&row.parameter_schema)?,
                secret_slot_schema: secret_slots(&row.secret_slot_schema)?,
                runtime_contract: contract(&row.runtime_contract, "")?.into(),
                requires_state: row.requires_state,
                update_hook: update_hook(row.update_hook.as_ref()).into(),
                release_id: opaque(row.release_id).into(),
                release_version: row.release_version,
                ..Default::default()
            })
        })
        .collect::<Result<Vec<_>, RpcError>>()?;
    let recent_runs = snapshot
        .recent_runs
        .into_iter()
        .map(|row| RecentRun {
            id: opaque(row.id).into(),
            state: row.state,
            outcome: row.outcome.unwrap_or_default(),
            run_kind: row.run_kind,
            instance_revision_id: opaque(row.instance_revision_id).into(),
            release_id: opaque(row.release_id).into(),
            attachment_id: row.attachment_id.map(opaque).into(),
            created_at: timestamp(row.created_at).into(),
            updated_at: timestamp(row.updated_at).into(),
            ..Default::default()
        })
        .collect();
    let row = snapshot.instance;
    Ok(AgentInstance {
        id: opaque(row.id).into(),
        name: row.name,
        state: row.state,
        run_gate_open: row.run_gate_open,
        active_revision_id: opaque(row.active_revision_id).into(),
        state_volume_id: row.state_volume_id.map(opaque).into(),
        created_at: timestamp(row.created_at).into(),
        updated_at: timestamp(row.updated_at).into(),
        project_id: opaque(row.project_id).into(),
        project_name: row.project_name,
        organization_id: opaque(row.organization_id).into(),
        organization_name: row.organization_name,
        can_manage: row.can_manage,
        can_update: row.can_update,
        can_recover: row.can_recover,
        revisions,
        attachments,
        updates,
        repositories,
        secret_imports,
        update_candidates,
        recent_runs,
        ..Default::default()
    })
}

fn opaque(value: Uuid) -> OpaqueId {
    OpaqueId {
        value: value.to_string(),
        ..Default::default()
    }
}
fn timestamp(value: OffsetDateTime) -> Timestamp {
    Timestamp {
        seconds: value.unix_timestamp(),
        nanos: value.nanosecond().cast_signed(),
        ..Default::default()
    }
}
fn string(value: &Value, key: &str) -> Result<String, RpcError> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or(RpcError::Internal)
}
fn boolean(value: &Value, key: &str) -> bool {
    value.get(key).and_then(Value::as_bool).unwrap_or(false)
}
fn unsigned(value: &Value, key: &str) -> Result<u32, RpcError> {
    value
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or(RpcError::Internal)
}

fn policy(value: &Value) -> Result<RuntimePolicy, RpcError> {
    Ok(RuntimePolicy {
        vcpus: unsigned(value, "vcpus")?,
        memory_mib: unsigned(value, "memory_mib")?,
        network: network(&string(value, "network")?)?.into(),
        ..Default::default()
    })
}
fn network(value: &str) -> Result<NetworkPolicy, RpcError> {
    match value {
        "disabled" => Ok(NetworkPolicy::Disabled),
        "broker_only" => Ok(NetworkPolicy::BrokerOnly),
        "egress" => Ok(NetworkPolicy::Egress),
        _ => Err(RpcError::Internal),
    }
}

fn network_restriction(value: &Value) -> Result<NetworkPolicy, RpcError> {
    network(&string(value, "network")?)
}

fn parameters(value: &Value) -> Result<Vec<ParameterValue>, RpcError> {
    value
        .as_object()
        .ok_or(RpcError::Internal)?
        .iter()
        .map(|(name, value)| {
            let value = if let Some(value) = value.as_bool() {
                parameter_value::Value::BooleanValue(value)
            } else if let Some(value) = value.as_i64() {
                parameter_value::Value::IntegerValue(value)
            } else {
                parameter_value::Value::StringValue(
                    value.as_str().ok_or(RpcError::Internal)?.to_owned(),
                )
            };
            Ok(ParameterValue {
                name: name.clone(),
                value: Some(value),
                ..Default::default()
            })
        })
        .collect()
}

fn parameter_schema(value: &Value) -> Result<Vec<ParameterDeclaration>, RpcError> {
    value
        .as_array()
        .ok_or(RpcError::Internal)?
        .iter()
        .map(|item| {
            let name = string(item, "name")?;
            let kind = item.get("value_type").ok_or(RpcError::Internal)?;
            let type_name = string(kind, "type")?;
            let constraint = match type_name.as_str() {
                "string" => {
                    parameter_type::Constraint::String(Box::new(StringParameterConstraints {
                        minimum_length: unsigned(kind, "minimum_length")?,
                        maximum_length: unsigned(kind, "maximum_length")?,
                        ..Default::default()
                    }))
                }
                "integer" => {
                    parameter_type::Constraint::Integer(Box::new(IntegerParameterConstraints {
                        minimum: kind
                            .get("minimum")
                            .and_then(Value::as_i64)
                            .ok_or(RpcError::Internal)?,
                        maximum: kind
                            .get("maximum")
                            .and_then(Value::as_i64)
                            .ok_or(RpcError::Internal)?,
                        ..Default::default()
                    }))
                }
                "boolean" => parameter_type::Constraint::Boolean(Box::default()),
                "enum" => {
                    parameter_type::Constraint::Enumeration(Box::new(EnumParameterConstraints {
                        values: kind
                            .get("values")
                            .and_then(Value::as_array)
                            .ok_or(RpcError::Internal)?
                            .iter()
                            .map(|value| {
                                value.as_str().map(str::to_owned).ok_or(RpcError::Internal)
                            })
                            .collect::<Result<Vec<_>, _>>()?,
                        ..Default::default()
                    }))
                }
                _ => return Err(RpcError::Internal),
            };
            let default = item
                .get("default")
                .filter(|value| !value.is_null())
                .map(parameter_default)
                .transpose()?
                .into();
            Ok(ParameterDeclaration {
                name: name.clone(),
                label: name,
                value_type: ParameterType {
                    constraint: Some(constraint),
                    ..Default::default()
                }
                .into(),
                required: boolean(item, "required"),
                default,
                sensitive: boolean(item, "sensitive"),
                ..Default::default()
            })
        })
        .collect()
}

fn parameter_default(value: &Value) -> Result<ParameterDefault, RpcError> {
    let value = if let Some(value) = value.as_bool() {
        parameter_default::Value::BooleanValue(value)
    } else if let Some(value) = value.as_i64() {
        parameter_default::Value::IntegerValue(value)
    } else {
        parameter_default::Value::StringValue(value.as_str().ok_or(RpcError::Internal)?.to_owned())
    };
    Ok(ParameterDefault {
        value: Some(value),
        ..Default::default()
    })
}

fn secret_slots(value: &Value) -> Result<Vec<SecretSlotDeclaration>, RpcError> {
    value
        .as_array()
        .ok_or(RpcError::Internal)?
        .iter()
        .map(|item| {
            Ok(SecretSlotDeclaration {
                key: string(item, "key")?,
                purpose: string(item, "purpose")?,
                required: boolean(item, "required"),
                delivery_modes: strings(item, "delivery_modes")?
                    .iter()
                    .map(|value| match value.as_str() {
                        "raw" => Ok(SecretSlotDeliveryMode::Raw.into()),
                        "brokered" => Ok(SecretSlotDeliveryMode::Brokered.into()),
                        _ => Err(RpcError::Internal),
                    })
                    .collect::<Result<Vec<_>, _>>()?,
                phases: strings(item, "phases")?
                    .iter()
                    .map(|value| match value.as_str() {
                        "normal" => Ok(SecretSlotPhase::Normal.into()),
                        "update" => Ok(SecretSlotPhase::Update.into()),
                        _ => Err(RpcError::Internal),
                    })
                    .collect::<Result<Vec<_>, _>>()?,
                destinations: strings(item, "destinations")?,
                ..Default::default()
            })
        })
        .collect()
}

fn strings(value: &Value, key: &str) -> Result<Vec<String>, RpcError> {
    value.get(key).and_then(Value::as_array).map_or_else(
        || Ok(Vec::new()),
        |items| {
            items
                .iter()
                .map(|item| item.as_str().map(str::to_owned).ok_or(RpcError::Internal))
                .collect()
        },
    )
}
fn contract(value: &Value, platform_policy_version: &str) -> Result<RuntimeContract, RpcError> {
    Ok(RuntimeContract {
        policy_ceiling: policy(value.get("policy_ceiling").ok_or(RpcError::Internal)?)?.into(),
        platform_policy_version: platform_policy_version.to_owned(),
        requires_state: boolean(value, "requires_state"),
        ..Default::default()
    })
}
fn update_hook(value: Option<&Value>) -> UpdateHook {
    UpdateHook {
        required: value.is_some(),
        timeout_seconds: value
            .and_then(|value| value.get("timeout_seconds"))
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .unwrap_or(0),
        ..Default::default()
    }
}

fn diagnostics(value: &Value) -> Result<Vec<Diagnostic>, RpcError> {
    let values = value.as_array().ok_or(RpcError::Internal)?;
    Ok(values.iter().map(diagnostic).collect())
}
fn diagnostic(value: &Value) -> Diagnostic {
    Diagnostic {
        code: diagnostic_code(value.get("code").and_then(Value::as_str).unwrap_or("")).into(),
        severity: DiagnosticSeverity::Error.into(),
        field: value
            .get("field")
            .or_else(|| value.get("path"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned(),
        message: value
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("")
            .chars()
            .take(4096)
            .collect(),
        ..Default::default()
    }
}
fn diagnostic_code(value: &str) -> DiagnosticCode {
    match value {
        "invalid_parameter" => DiagnosticCode::InvalidParameter,
        "policy_exceeded" => DiagnosticCode::PolicyExceeded,
        "incompatible_update" => DiagnosticCode::IncompatibleUpdate,
        "run_gate_closed" => DiagnosticCode::RunGateClosed,
        "resource_unavailable" => DiagnosticCode::ResourceUnavailable,
        _ => DiagnosticCode::Unspecified,
    }
}

fn selector(value: &str) -> RefSelector {
    let selector = value.strip_suffix("/*").map_or_else(
        || ref_selector::Selector::Exact(value.to_owned()),
        |prefix| ref_selector::Selector::Prefix(prefix.to_owned()),
    );
    RefSelector {
        selector: Some(selector),
        ..Default::default()
    }
}
fn trigger(value: &str) -> Result<TriggerPolicy, RpcError> {
    match value {
        "manual" => Ok(TriggerPolicy::Manual),
        "push" => Ok(TriggerPolicy::Push),
        "push_and_manual" => Ok(TriggerPolicy::PushAndManual),
        _ => Err(RpcError::Internal),
    }
}
fn recovery(value: Option<&str>) -> RecoveryDecision {
    match value {
        Some("agent_rejected" | "rejected") => RecoveryDecision::Rejected,
        Some("retry" | "retry_queued") => RecoveryDecision::RetryQueued,
        Some("resumed" | "activated") => RecoveryDecision::Resumed,
        _ => RecoveryDecision::Unspecified,
    }
}

fn events(value: &Value) -> Result<Vec<UpdateEvent>, RpcError> {
    let values = value.as_array().ok_or(RpcError::Internal)?;
    if values.len() > MAX_HOOK_EVENTS {
        return Err(RpcError::ResourceExhausted);
    }
    values
        .iter()
        .map(|item| {
            let sequence = item
                .get("sequence")
                .and_then(Value::as_u64)
                .ok_or(RpcError::Internal)?;
            let event_type = string(item, "event_type")?;
            let body = item.get("payload").unwrap_or(&Value::Null);
            let payload = match event_type.as_str() {
                "vm.log" => body.get("message").and_then(Value::as_str).map(|value| {
                    update_event::Payload::BoundedLogMessage(value.chars().take(4096).collect())
                }),
                "diagnostic" => Some(update_event::Payload::Diagnostic(Box::new(diagnostic(
                    body,
                )))),
                "operation.state" => body.get("state").and_then(Value::as_str).map(|state| {
                    update_event::Payload::OperationState(operation_state(state).into())
                }),
                _ => None,
            };
            Ok(UpdateEvent {
                sequence,
                event_type,
                payload,
                ..Default::default()
            })
        })
        .collect()
}
fn operation_state(value: &str) -> OperationState {
    match value {
        "queued" => OperationState::Queued,
        "running" => OperationState::Running,
        "succeeded" => OperationState::Succeeded,
        "failed" => OperationState::Failed,
        "cancelled" => OperationState::Cancelled,
        _ => OperationState::Unspecified,
    }
}

fn target(kind: &str, id: Uuid) -> Result<SecretTarget, RpcError> {
    let target = match kind {
        "project" => secret_target::Target::ProjectId(Box::new(opaque(id))),
        "repository" => secret_target::Target::RepositoryId(Box::new(opaque(id))),
        _ => return Err(RpcError::Internal),
    };
    Ok(SecretTarget {
        target: Some(target),
        ..Default::default()
    })
}
fn authority(value: &str) -> Result<AuthorityState, RpcError> {
    match value {
        "active" => Ok(AuthorityState::Active),
        "revoked" => Ok(AuthorityState::Revoked),
        "expired" => Ok(AuthorityState::Expired),
        _ => Err(RpcError::Internal),
    }
}
fn secret_state(value: &str) -> Result<SecretState, RpcError> {
    match value {
        "active" => Ok(SecretState::Active),
        "disabled" => Ok(SecretState::Disabled),
        "revoked" => Ok(SecretState::Revoked),
        "purged" => Ok(SecretState::Purged),
        _ => Err(RpcError::Internal),
    }
}
fn delivery(value: &str) -> Result<buffa::EnumValue<DeliveryMode>, RpcError> {
    match value {
        "raw" => Ok(DeliveryMode::Raw.into()),
        "brokered" => Ok(DeliveryMode::Brokered.into()),
        _ => Err(RpcError::Internal),
    }
}
fn phase(value: &str) -> Result<buffa::EnumValue<DeliveryPhase>, RpcError> {
    match value {
        "normal" => Ok(DeliveryPhase::Normal.into()),
        "update" => Ok(DeliveryPhase::Update.into()),
        _ => Err(RpcError::Internal),
    }
}
fn encode_hex(value: &[u8]) -> String {
    value.iter().fold(
        String::with_capacity(value.len() * 2),
        |mut output, byte| {
            use std::fmt::Write as _;
            let _ = write!(output, "{byte:02x}");
            output
        },
    )
}

fn map_error(error: InstanceQueryError) -> RpcError {
    match error {
        InstanceQueryError::PermissionDenied => RpcError::PermissionDenied,
        InstanceQueryError::NotFound => RpcError::NotFound,
        InstanceQueryError::ResponseTooLarge => RpcError::ResourceExhausted,
        InstanceQueryError::Persistence(source) => {
            tracing::error!(error = %source, "agent-instance query failed");
            RpcError::Unavailable
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_HOOK_EVENTS, MAX_RESPONSE_BYTES, ensure_response_bound, events, network_restriction,
        parameter_schema, selector,
    };
    use rpc_proto::messages::hephaestus::{
        common::v1::{NetworkPolicy, parameter_type},
        instance::v1::{AgentInstance, GetInstanceResponse, ref_selector, update_event},
    };
    use serde_json::json;

    #[test]
    fn stored_documents_become_generated_contract_types() {
        let declarations = parameter_schema(&json!([{
            "name": "count",
            "value_type": {"type": "integer", "minimum": 1, "maximum": 4},
            "required": true,
            "sensitive": false
        }]))
        .expect("parameter schema");
        let constraint = declarations[0]
            .value_type
            .as_option()
            .and_then(|value| value.constraint.as_ref())
            .expect("parameter constraint");
        assert!(matches!(
            constraint,
            parameter_type::Constraint::Integer(value)
                if value.minimum == 1 && value.maximum == 4
        ));

        let selector = selector("refs/heads/*");
        assert!(matches!(
            selector.selector,
            Some(ref_selector::Selector::Prefix(value)) if value == "refs/heads"
        ));
        assert_eq!(
            network_restriction(&json!({"network": "broker_only"})).expect("network"),
            NetworkPolicy::BrokerOnly
        );
    }

    #[test]
    fn update_logs_are_bounded_to_the_contract_limit() {
        let parsed_events = events(
            &json!([{"sequence":1,"event_type":"vm.log","payload":{"message":"x".repeat(5000)}}]),
        )
        .expect("events");
        match parsed_events[0].payload.as_ref().expect("payload") {
            update_event::Payload::BoundedLogMessage(value) => assert_eq!(value.len(), 4096),
            _ => panic!("unexpected event"),
        }

        let excessive = serde_json::Value::Array(
            (0..=MAX_HOOK_EVENTS)
                .map(|sequence| {
                    json!({"sequence": sequence, "event_type": "operation.state", "payload": {"state": "running"}})
                })
                .collect(),
        );
        assert!(events(&excessive).is_err());
    }

    #[test]
    fn response_enforces_the_generated_four_mebibyte_limit() {
        let response = GetInstanceResponse {
            instance: AgentInstance {
                name: "x".repeat(usize::try_from(MAX_RESPONSE_BYTES).expect("response bound") + 1),
                ..Default::default()
            }
            .into(),
            ..Default::default()
        };
        assert!(ensure_response_bound(&response).is_err());
    }
}
