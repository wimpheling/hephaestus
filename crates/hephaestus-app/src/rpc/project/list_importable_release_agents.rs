use super::{ProjectRpc, map_error, opaque, parse_id, parse_page};
use crate::{
    application::project::ReleaseAgentRow,
    rpc::{RpcError, into_connect_error, request},
};
use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult};
use rpc_proto::messages::hephaestus::{
    common::v1::{
        BooleanParameterConstraints, EnumParameterConstraints, IntegerParameterConstraints,
        NetworkPolicy, PageResponse, ParameterDeclaration, ParameterDefault,
        ParameterType as ProtoParameterType, RuntimeContract, RuntimePolicy, SecretSlotDeclaration,
        SecretSlotDeliveryMode, SecretSlotPhase, StringParameterConstraints, parameter_default,
        parameter_type,
    },
    project::v1::{
        ListImportableReleaseAgentsRequest, ListImportableReleaseAgentsResponse, ReleaseAgentOption,
    },
};

pub(super) async fn handle(
    service: &ProjectRpc,
    ctx: RequestContext,
    message: ServiceRequest<'_, ListImportableReleaseAgentsRequest>,
) -> ServiceResult<ListImportableReleaseAgentsResponse> {
    let identity = request::query_identity(
        &ctx,
        &service.authenticator,
        "/hephaestus.project.v1.ProjectService/ListImportableReleaseAgents",
    )
    .map_err(into_connect_error)?;
    let request = message.to_owned_message();
    let project_id = parse_id(request.project_id.as_option()).map_err(into_connect_error)?;
    let page = parse_page(request.page.as_option()).map_err(into_connect_error)?;
    let result = service
        .application
        .importable_agents(&identity, project_id, page)
        .await
        .map_err(map_error)
        .map_err(into_connect_error)?;
    let release_agents = result
        .values
        .into_iter()
        .map(release_agent_option)
        .collect::<Result<Vec<_>, crate::rpc::RpcError>>()
        .map_err(into_connect_error)?;
    Response::ok(ListImportableReleaseAgentsResponse {
        release_agents,
        page: PageResponse {
            next_page_token: result.next.unwrap_or_default(),
            stable_order: String::from("id"),
            ..Default::default()
        }
        .into(),
        ..Default::default()
    })
}

fn release_agent_option(row: ReleaseAgentRow) -> Result<ReleaseAgentOption, RpcError> {
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
        ..Default::default()
    })
}

fn parameter_schema(value: &serde_json::Value) -> Result<Vec<ParameterDeclaration>, RpcError> {
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

fn secret_slot_schema(value: &serde_json::Value) -> Result<Vec<SecretSlotDeclaration>, RpcError> {
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

fn runtime_contract(
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

#[cfg(test)]
mod tests {
    use super::{parameter_schema, runtime_contract, secret_slot_schema};
    use rpc_proto::messages::hephaestus::common::v1::{
        NetworkPolicy, parameter_default, parameter_type,
    };
    use serde_json::json;

    #[test]
    fn durable_release_documents_preserve_typed_defaults_and_lists() {
        let parameters = parameter_schema(&json!([
            {
                "name": "review_style",
                "value_type": {"type": "enum", "values": ["strict", "balanced"]},
                "required": true,
                "default": "balanced",
                "sensitive": false
            },
            {
                "name": "attempts",
                "value_type": {"type": "integer", "minimum": 1, "maximum": 4},
                "required": false,
                "default": 2,
                "sensitive": false
            },
            {
                "name": "enabled",
                "value_type": {"type": "boolean"},
                "required": false,
                "default": true,
                "sensitive": false
            }
        ]))
        .expect("durable parameter schema");

        assert_eq!(parameters[0].name, "review_style");
        assert!(matches!(
            parameters[0]
                .value_type
                .as_option()
                .and_then(|value| value.constraint.as_ref()),
            Some(parameter_type::Constraint::Enumeration(value))
                if value.values == ["strict", "balanced"]
        ));
        assert!(matches!(
            parameters[0]
                .default
                .as_option()
                .and_then(|value| value.value.as_ref()),
            Some(parameter_default::Value::StringValue(value)) if value == "balanced"
        ));
        assert!(matches!(
            parameters[1]
                .default
                .as_option()
                .and_then(|value| value.value.as_ref()),
            Some(parameter_default::Value::IntegerValue(2))
        ));
        assert!(matches!(
            parameters[2]
                .default
                .as_option()
                .and_then(|value| value.value.as_ref()),
            Some(parameter_default::Value::BooleanValue(true))
        ));

        let slots = secret_slot_schema(&json!([{
            "key": "review_token",
            "purpose": "Review API",
            "required": true,
            "delivery_modes": ["raw", "brokered"],
            "phases": ["normal", "update"],
            "destinations": ["reviews.submit"]
        }]))
        .expect("durable secret slot schema");
        assert_eq!(slots[0].delivery_modes.len(), 2);
        assert_eq!(slots[0].phases.len(), 2);
        assert_eq!(slots[0].destinations, ["reviews.submit"]);

        let contract = runtime_contract(
            &json!({
                "policy_ceiling": {
                    "vcpus": 1,
                    "memory_mib": 128,
                    "network": "broker_only"
                }
            }),
            true,
        )
        .expect("durable runtime contract");
        assert_eq!(
            contract
                .policy_ceiling
                .as_option()
                .map(|policy| policy.network),
            Some(NetworkPolicy::BrokerOnly.into())
        );
        assert!(contract.requires_state);
    }
}
