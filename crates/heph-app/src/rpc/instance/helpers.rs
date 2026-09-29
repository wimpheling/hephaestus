use super::super::{MediatorAuthenticator, RpcError};
use crate::rpc::{into_connect_error, request};
use capability_domain::{CapabilityOperation, CapabilityResourceKind};
use connectrpc::RequestContext;
use release_domain::{NetworkAccess, ParameterName, ParameterValue, RuntimePolicy};
use release_postgres::BrokeredRuleCopy;
use rpc_proto::messages::hephaestus::{
    common::v1::{
        NetworkPolicy, OpaqueId, ParameterValue as ProtoParameterValue,
        RuntimePolicy as ProtoRuntimePolicy, parameter_value,
    },
    instance::v1::DeclareBrokeredHttpsRuleRequest,
};
use serde_json::Value;
use std::{collections::BTreeMap, str::FromStr};
use uuid::Uuid;

pub(super) fn capability_resource_kind(
    value: &str,
) -> Result<CapabilityResourceKind, connectrpc::ConnectError> {
    match value {
        "repository" => Ok(CapabilityResourceKind::Repository),
        "project" => Ok(CapabilityResourceKind::Project),
        "agent_instance" => Ok(CapabilityResourceKind::AgentInstance),
        "gateway" => Ok(CapabilityResourceKind::Gateway),
        "run" => Ok(CapabilityResourceKind::Run),
        "state_volume" => Ok(CapabilityResourceKind::StateVolume),
        _ => Err(into_connect_error(RpcError::InvalidArgument)),
    }
}

pub(super) fn capability_operation(
    value: &str,
) -> Result<CapabilityOperation, connectrpc::ConnectError> {
    let operation = match value {
        "inspect" => CapabilityOperation::Inspect,
        "configure" => CapabilityOperation::Configure,
        "execute" => CapabilityOperation::Execute,
        "update" => CapabilityOperation::Update,
        "pause" => CapabilityOperation::Pause,
        "recover" => CapabilityOperation::Recover,
        "cancel" => CapabilityOperation::Cancel,
        "attach" => CapabilityOperation::Attach,
        "restore" => CapabilityOperation::Restore,
        "git_read" => CapabilityOperation::GitRead,
        "create_ref" => CapabilityOperation::CreateRef,
        "update_ref" => CapabilityOperation::UpdateRef,
        "force_update_ref" => CapabilityOperation::ForceUpdateRef,
        "delete_ref" => CapabilityOperation::DeleteRef,
        "create_tag" => CapabilityOperation::CreateTag,
        "delete_tag" => CapabilityOperation::DeleteTag,
        "trigger_run" => CapabilityOperation::TriggerRun,
        "manage_attachments" => CapabilityOperation::ManageAttachments,
        _ => return Err(into_connect_error(RpcError::InvalidArgument)),
    };
    Ok(operation)
}

pub(super) fn mutation(
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

pub(super) fn parse_id<T: FromStr>(
    value: Option<&OpaqueId>,
) -> Result<T, connectrpc::ConnectError> {
    request::required_id(value)
        .map_err(into_connect_error)?
        .parse()
        .map_err(|_| into_connect_error(RpcError::InvalidArgument))
}

pub(super) fn requested_rule_id(
    request: &DeclareBrokeredHttpsRuleRequest,
) -> Result<Option<Uuid>, connectrpc::ConnectError> {
    request
        .requested_rule_id
        .as_option()
        .map(|value| parse_id(Some(value)))
        .transpose()
}

pub(super) fn policy(
    value: Option<&ProtoRuntimePolicy>,
) -> Result<RuntimePolicy, connectrpc::ConnectError> {
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

pub(super) fn parameters(
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

pub(super) fn brokered_rule_copies(
    values: Vec<rpc_proto::messages::hephaestus::instance::v1::BrokeredRuleCopy>,
) -> Result<Vec<BrokeredRuleCopy>, connectrpc::ConnectError> {
    values
        .into_iter()
        .map(|value| {
            Ok(BrokeredRuleCopy {
                source_rule_id: parse_id(value.source_rule_id.as_option())?,
                candidate_rule_id: parse_id(value.candidate_rule_id.as_option())?,
            })
        })
        .collect()
}

pub(super) fn json_id(value: &Value, field: &str) -> Result<String, connectrpc::ConnectError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| into_connect_error(RpcError::Internal))
}

pub(super) fn opaque(value: String) -> OpaqueId {
    OpaqueId {
        value,
        ..Default::default()
    }
}

pub(super) fn invalid<T>(_error: T) -> connectrpc::ConnectError {
    into_connect_error(RpcError::InvalidArgument)
}

#[cfg(test)]
mod tests {
    use super::requested_rule_id;
    use rpc_proto::messages::hephaestus::{
        common::v1::OpaqueId, instance::v1::DeclareBrokeredHttpsRuleRequest,
    };
    use uuid::Uuid;

    #[test]
    fn requested_rule_id_is_optional_but_must_be_a_uuid_when_present() {
        let absent = DeclareBrokeredHttpsRuleRequest::default();
        assert_eq!(requested_rule_id(&absent).expect("absent is valid"), None);

        let expected = Uuid::new_v4();
        let valid = DeclareBrokeredHttpsRuleRequest {
            requested_rule_id: OpaqueId {
                value: expected.to_string(),
                ..Default::default()
            }
            .into(),
            ..Default::default()
        };
        assert_eq!(
            requested_rule_id(&valid).expect("valid UUID is accepted"),
            Some(expected)
        );

        let malformed = DeclareBrokeredHttpsRuleRequest {
            requested_rule_id: OpaqueId {
                value: "rule-from-client".to_owned(),
                ..Default::default()
            }
            .into(),
            ..Default::default()
        };
        assert!(requested_rule_id(&malformed).is_err());
    }
}
