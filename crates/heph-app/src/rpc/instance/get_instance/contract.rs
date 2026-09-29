use super::MAX_HOOK_EVENTS;
use super::policy::operation_state;
use crate::rpc::RpcError;
use rpc_proto::messages::hephaestus::{
    common::v1::{
        Diagnostic, DiagnosticCode, DiagnosticSeverity, EnumParameterConstraints,
        IntegerParameterConstraints, NetworkPolicy, ParameterDeclaration, ParameterDefault,
        ParameterType, ParameterValue, RuntimeContract, RuntimePolicy, SecretSlotDeclaration,
        SecretSlotDeliveryMode, SecretSlotPhase, StringParameterConstraints, UpdateHook,
        parameter_default, parameter_type, parameter_value,
    },
    instance::v1::{
        RecoveryDecision, RefSelector, TriggerPolicy, UpdateEvent, ref_selector, update_event,
    },
};
use serde_json::Value;

pub(super) fn string(value: &Value, key: &str) -> Result<String, RpcError> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or(RpcError::Internal)
}

fn boolean(value: &Value, key: &str) -> bool {
    value.get(key).and_then(Value::as_bool).unwrap_or(false)
}
pub(super) fn unsigned(value: &Value, key: &str) -> Result<u32, RpcError> {
    value
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or(RpcError::Internal)
}

pub(super) fn policy(value: &Value) -> Result<RuntimePolicy, RpcError> {
    Ok(RuntimePolicy {
        vcpus: unsigned(value, "vcpus")?,
        memory_mib: unsigned(value, "memory_mib")?,
        network: network(&string(value, "network")?)?.into(),
        ..Default::default()
    })
}
pub(super) fn network(value: &str) -> Result<NetworkPolicy, RpcError> {
    match value {
        "disabled" => Ok(NetworkPolicy::Disabled),
        "broker_only" => Ok(NetworkPolicy::BrokerOnly),
        "egress" => Ok(NetworkPolicy::Egress),
        _ => Err(RpcError::Internal),
    }
}

pub(super) fn network_restriction(value: &Value) -> Result<NetworkPolicy, RpcError> {
    network(&string(value, "network")?)
}

pub(super) fn parameters(value: &Value) -> Result<Vec<ParameterValue>, RpcError> {
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

pub(super) fn parameter_schema(value: &Value) -> Result<Vec<ParameterDeclaration>, RpcError> {
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

pub(super) fn parameter_default(value: &Value) -> Result<ParameterDefault, RpcError> {
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

pub(super) fn secret_slots(value: &Value) -> Result<Vec<SecretSlotDeclaration>, RpcError> {
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

pub(super) fn strings(value: &Value, key: &str) -> Result<Vec<String>, RpcError> {
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
pub(super) fn contract(
    value: &Value,
    platform_policy_version: &str,
) -> Result<RuntimeContract, RpcError> {
    Ok(RuntimeContract {
        policy_ceiling: policy(value.get("policy_ceiling").ok_or(RpcError::Internal)?)?.into(),
        platform_policy_version: platform_policy_version.to_owned(),
        requires_state: boolean(value, "requires_state"),
        ..Default::default()
    })
}
pub(super) fn update_hook(value: Option<&Value>) -> UpdateHook {
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

pub(super) fn diagnostics(value: &Value) -> Result<Vec<Diagnostic>, RpcError> {
    let values = value.as_array().ok_or(RpcError::Internal)?;
    Ok(values.iter().map(diagnostic).collect())
}
pub(super) fn diagnostic(value: &Value) -> Diagnostic {
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
pub(super) fn diagnostic_code(value: &str) -> DiagnosticCode {
    match value {
        "invalid_parameter" => DiagnosticCode::InvalidParameter,
        "policy_exceeded" => DiagnosticCode::PolicyExceeded,
        "incompatible_update" => DiagnosticCode::IncompatibleUpdate,
        "run_gate_closed" => DiagnosticCode::RunGateClosed,
        "resource_unavailable" => DiagnosticCode::ResourceUnavailable,
        _ => DiagnosticCode::Unspecified,
    }
}

pub(super) fn selector(value: &str) -> RefSelector {
    let selector = value.strip_suffix("/*").map_or_else(
        || ref_selector::Selector::Exact(value.to_owned()),
        |prefix| ref_selector::Selector::Prefix(prefix.to_owned()),
    );
    RefSelector {
        selector: Some(selector),
        ..Default::default()
    }
}
pub(super) fn trigger(value: &str) -> Result<TriggerPolicy, RpcError> {
    match value {
        "manual" => Ok(TriggerPolicy::Manual),
        "push" => Ok(TriggerPolicy::Push),
        "push_and_manual" => Ok(TriggerPolicy::PushAndManual),
        _ => Err(RpcError::Internal),
    }
}
pub(super) fn recovery(value: Option<&str>) -> RecoveryDecision {
    match value {
        Some("agent_rejected" | "rejected") => RecoveryDecision::Rejected,
        Some("retry" | "retry_queued") => RecoveryDecision::RetryQueued,
        Some("resumed" | "activated") => RecoveryDecision::Resumed,
        _ => RecoveryDecision::Unspecified,
    }
}

pub(super) fn events(value: &Value) -> Result<Vec<UpdateEvent>, RpcError> {
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
