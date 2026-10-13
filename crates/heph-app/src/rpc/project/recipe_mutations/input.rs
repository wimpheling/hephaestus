use crate::rpc::{RpcError, project::parse_id};
use capability_domain::CapabilitySlotKey;
use heph_forge::ProjectId;
use recipe_application::{DeploymentId, DeploymentKey, PlanningRequest};
use release_domain::{ParameterName, ParameterValue};
use rpc_proto::messages::hephaestus::{
    common::v1::{OpaqueId, parameter_value::Value},
    project::v1::{InstallRecipeDeploymentRequest, ResumeRecipeDeploymentRequest},
};
use runtime_types::VolumeId;
use std::collections::BTreeMap;

pub(super) fn id(value: Option<&OpaqueId>) -> Result<uuid::Uuid, RpcError> {
    let value = value.ok_or(RpcError::InvalidArgument)?;
    if !value.__buffa_unknown_fields.is_empty() {
        return Err(RpcError::InvalidArgument);
    }
    let id = parse_id(Some(value))?;
    if id.is_nil() {
        return Err(RpcError::InvalidArgument);
    }
    Ok(id)
}
pub(super) fn install(value: &InstallRecipeDeploymentRequest) -> Result<PlanningRequest, RpcError> {
    if !value.__buffa_unknown_fields.is_empty()
        || value.declaration_toml.len() > 65536
        || value.deployment_key.len() > 128
        || value.inputs.len() > 64
        || value.external_volumes.len() > 64
    {
        return Err(RpcError::InvalidArgument);
    }
    let project = ProjectId::from_uuid(id(value.project_id.as_option())?);
    let key = DeploymentKey::parse(value.deployment_key.clone())
        .map_err(|_| RpcError::InvalidArgument)?;
    let mut inputs = BTreeMap::new();
    for parameter in &value.inputs {
        if !parameter.__buffa_unknown_fields.is_empty() {
            return Err(RpcError::InvalidArgument);
        }
        let name =
            ParameterName::parse(parameter.name.clone()).map_err(|_| RpcError::InvalidArgument)?;
        let item = match parameter.value.as_ref().ok_or(RpcError::InvalidArgument)? {
            Value::StringValue(text) if text.len() <= 4096 => ParameterValue::String(text.clone()),
            Value::IntegerValue(value) => ParameterValue::Integer(*value),
            Value::BooleanValue(value) => ParameterValue::Boolean(*value),
            Value::StringValue(_) => return Err(RpcError::InvalidArgument),
        };
        if inputs.contains_key(&name) {
            return Err(RpcError::InvalidArgument);
        }
        inputs.insert(name, item);
    }
    let mut external = BTreeMap::new();
    for binding in &value.external_volumes {
        if !binding.__buffa_unknown_fields.is_empty() {
            return Err(RpcError::InvalidArgument);
        }
        let name = CapabilitySlotKey::parse(binding.name.clone())
            .map_err(|_| RpcError::InvalidArgument)?;
        let volume = VolumeId::from_uuid(id(binding.volume_id.as_option())?);
        if external.contains_key(&name) {
            return Err(RpcError::InvalidArgument);
        }
        external.insert(name, volume);
    }
    PlanningRequest::new(
        project,
        key,
        value.declaration_toml.as_bytes(),
        inputs,
        external,
    )
    .map_err(|_| RpcError::InvalidArgument)
}
pub(super) fn resume(
    value: &ResumeRecipeDeploymentRequest,
) -> Result<(ProjectId, DeploymentId), RpcError> {
    if !value.__buffa_unknown_fields.is_empty() {
        return Err(RpcError::InvalidArgument);
    }
    Ok((
        ProjectId::from_uuid(id(value.project_id.as_option())?),
        DeploymentId::from_uuid(id(value.deployment_id.as_option())?)
            .map_err(|_| RpcError::InvalidArgument)?,
    ))
}
