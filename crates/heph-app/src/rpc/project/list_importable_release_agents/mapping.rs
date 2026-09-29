use super::opaque;
use crate::{application::project::ReleaseAgentRow, rpc::RpcError};
use rpc_proto::messages::hephaestus::{
    common::v1::{
        BooleanParameterConstraints, EnumParameterConstraints, IntegerParameterConstraints,
        NetworkPolicy, ParameterDeclaration, ParameterDefault, ParameterType as ProtoParameterType,
        RuntimeContract, RuntimePolicy, SecretSlotDeclaration, SecretSlotDeliveryMode,
        SecretSlotPhase, StringParameterConstraints, parameter_default, parameter_type,
    },
    instance::v1::CapabilityRequirement,
    project::v1::ReleaseAgentOption,
};

pub(super) fn release_agent_option(row: ReleaseAgentRow) -> Result<ReleaseAgentOption, RpcError> {
    Ok(ReleaseAgentOption {
        id: opaque(row.id).into(),
        display_name: row.display_name,
        parameter_schema: parameter_schema(&row.parameter_schema)?,
        secret_slot_schema: secret_slot_schema(&row.secret_slot_schema)?,
        runtime_contract: runtime_contract(&row.runtime_contract, row.requires_state)?.into(),
        requires_state: row.requires_state,
        release_id: opaque(row.release_id).into(),
        release_version: row.release_version,
        source_commit: row.source_commit,
        repository_id: opaque(row.repository_id).into(),
        repository_name: row.repository_name,
        capability_requirements: capability_requirements(&row.capability_requirements)?,
        ..Default::default()
    })
}

pub(super) fn capability_requirements(
    value: &serde_json::Value,
) -> Result<Vec<CapabilityRequirement>, RpcError> {
    value
        .as_array()
        .ok_or(RpcError::Internal)?
        .iter()
        .map(|item| {
            Ok(CapabilityRequirement {
                id: opaque(uuid(item, "id")?).into(),
                release_agent_id: opaque(uuid(item, "release_agent_id")?).into(),
                slot_key: string(item, "slot_key")?,
                purpose: string(item, "purpose")?,
                resource_kind: string(item, "resource_kind")?,
                required_operations: strings(item, "required_operations")?,
                optional_operations: strings(item, "optional_operations")?,
                slot_required: boolean(item, "slot_required"),
                ..Default::default()
            })
        })
        .collect()
}

fn uuid(value: &serde_json::Value, key: &str) -> Result<uuid::Uuid, RpcError> {
    value
        .get(key)
        .and_then(serde_json::Value::as_str)
        .and_then(|value| uuid::Uuid::parse_str(value).ok())
        .ok_or(RpcError::Internal)
}

pub(super) fn parameter_schema(
    value: &serde_json::Value,
) -> Result<Vec<ParameterDeclaration>, RpcError> {
    value
        .as_array()
        .ok_or(RpcError::Internal)?
        .iter()
        .map(parameter)
        .collect()
}

fn parameter(value: &serde_json::Value) -> Result<ParameterDeclaration, RpcError> {
    let name = string(value, "name")?;
    let kind = value.get("value_type").ok_or(RpcError::Internal)?;
    let constraint = match string(kind, "type")?.as_str() {
        "string" => parameter_type::Constraint::String(Box::new(StringParameterConstraints {
            minimum_length: unsigned(kind, "minimum_length")?,
            maximum_length: unsigned(kind, "maximum_length")?,
            ..Default::default()
        })),
        "integer" => parameter_type::Constraint::Integer(Box::new(IntegerParameterConstraints {
            minimum: integer(kind, "minimum")?,
            maximum: integer(kind, "maximum")?,
            ..Default::default()
        })),
        "boolean" => {
            parameter_type::Constraint::Boolean(Box::<BooleanParameterConstraints>::default())
        }
        "enum" => parameter_type::Constraint::Enumeration(Box::new(EnumParameterConstraints {
            values: strings(kind, "values")?,
            ..Default::default()
        })),
        _ => return Err(RpcError::Internal),
    };
    let default = value
        .get("default")
        .filter(|value| !value.is_null())
        .map(parameter_default)
        .transpose()?
        .into();
    Ok(ParameterDeclaration {
        name: name.clone(),
        label: name,
        value_type: ProtoParameterType {
            constraint: Some(constraint),
            ..Default::default()
        }
        .into(),
        required: boolean(value, "required"),
        default,
        sensitive: boolean(value, "sensitive"),
        ..Default::default()
    })
}

pub(super) fn secret_slot_schema(
    value: &serde_json::Value,
) -> Result<Vec<SecretSlotDeclaration>, RpcError> {
    value
        .as_array()
        .ok_or(RpcError::Internal)?
        .iter()
        .map(secret_slot)
        .collect()
}

fn secret_slot(value: &serde_json::Value) -> Result<SecretSlotDeclaration, RpcError> {
    Ok(SecretSlotDeclaration {
        key: string(value, "key")?,
        purpose: string(value, "purpose")?,
        required: boolean(value, "required"),
        delivery_modes: strings(value, "delivery_modes")?
            .iter()
            .map(|mode| match mode.as_str() {
                "raw" => Ok(SecretSlotDeliveryMode::Raw.into()),
                "brokered" => Ok(SecretSlotDeliveryMode::Brokered.into()),
                _ => Err(RpcError::Internal),
            })
            .collect::<Result<Vec<_>, _>>()?,
        phases: strings(value, "phases")?
            .iter()
            .map(|phase| match phase.as_str() {
                "normal" => Ok(SecretSlotPhase::Normal.into()),
                "update" => Ok(SecretSlotPhase::Update.into()),
                _ => Err(RpcError::Internal),
            })
            .collect::<Result<Vec<_>, _>>()?,
        destinations: strings(value, "destinations")?,
        ..Default::default()
    })
}

pub(super) fn runtime_contract(
    value: &serde_json::Value,
    requires_state: bool,
) -> Result<RuntimeContract, RpcError> {
    let policy = value.get("policy_ceiling").ok_or(RpcError::Internal)?;
    Ok(RuntimeContract {
        policy_ceiling: RuntimePolicy {
            vcpus: unsigned(policy, "vcpus")?,
            memory_mib: unsigned(policy, "memory_mib")?,
            network: match string(policy, "network")?.as_str() {
                "disabled" => NetworkPolicy::Disabled,
                "broker_only" => NetworkPolicy::BrokerOnly,
                "egress" => NetworkPolicy::Egress,
                _ => return Err(RpcError::Internal),
            }
            .into(),
            ..Default::default()
        }
        .into(),
        requires_state,
        ..Default::default()
    })
}

fn parameter_default(value: &serde_json::Value) -> Result<ParameterDefault, RpcError> {
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

fn string(value: &serde_json::Value, key: &str) -> Result<String, RpcError> {
    value
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .ok_or(RpcError::Internal)
}

fn strings(value: &serde_json::Value, key: &str) -> Result<Vec<String>, RpcError> {
    value
        .get(key)
        .and_then(serde_json::Value::as_array)
        .map_or_else(
            || Ok(Vec::new()),
            |items| {
                items
                    .iter()
                    .map(|item| item.as_str().map(str::to_owned).ok_or(RpcError::Internal))
                    .collect()
            },
        )
}

fn boolean(value: &serde_json::Value, key: &str) -> bool {
    value
        .get(key)
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

fn unsigned(value: &serde_json::Value, key: &str) -> Result<u32, RpcError> {
    value
        .get(key)
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or(RpcError::Internal)
}

fn integer(value: &serde_json::Value, key: &str) -> Result<i64, RpcError> {
    value
        .get(key)
        .and_then(serde_json::Value::as_i64)
        .ok_or(RpcError::Internal)
}
