//! Closed comparison wires from native142; no wire confers authority.
use super::{DeploymentError, Value};
use crate::effect_state::State;
use identity_domain::{RequestId, UserId};
use recipe_application::{CommandIdentity, DeploymentOperation};
use uuid::Uuid;

pub(super) fn fields(value: &Value, names: &[&str]) -> Result<(), DeploymentError> {
    if value.as_object().is_none_or(|object| {
        object.len() != names.len() || names.iter().any(|name| !object.contains_key(*name))
    }) {
        return Err(DeploymentError::ReconciliationRequired);
    }
    Ok(())
}

pub(super) fn array<'a>(
    value: &'a Value,
    name: &str,
    bound: usize,
) -> Result<&'a [Value], DeploymentError> {
    let values = value
        .get(name)
        .and_then(Value::as_array)
        .ok_or(DeploymentError::ReconciliationRequired)?;
    if values.len() > bound {
        return Err(DeploymentError::ReconciliationRequired);
    }
    Ok(values)
}

pub(super) fn optional<'a>(
    value: &'a Value,
    name: &str,
    bound: usize,
) -> Result<&'a [Value], DeploymentError> {
    if value.get(name).is_none() {
        return Ok(&[]);
    }
    let values = array(value, name, bound)?;
    if values.is_empty() {
        return Err(DeploymentError::ReconciliationRequired);
    }
    Ok(values)
}

pub(super) fn uuid(value: &Value, name: &str) -> Result<Uuid, DeploymentError> {
    let id = value
        .get(name)
        .and_then(Value::as_str)
        .ok_or(DeploymentError::ReconciliationRequired)?
        .parse::<Uuid>()
        .map_err(|_| DeploymentError::ReconciliationRequired)?;
    if id.is_nil() {
        return Err(DeploymentError::ReconciliationRequired);
    }
    Ok(id)
}

pub(super) fn command(
    value: &Value,
    operation: DeploymentOperation,
) -> Result<CommandIdentity, DeploymentError> {
    CommandIdentity::from_recorded(
        UserId::from_uuid(uuid(value, "actor")?),
        RequestId::from_uuid(uuid(value, "idempotency")?),
        operation,
    )
}

pub(super) fn state(value: &Value) -> Result<State, DeploymentError> {
    fields(
        value,
        &[
            "install",
            "removal",
            "version",
            "active_attempt",
            "diagnostic",
            "generation",
            "last_action",
        ],
    )?;
    let state: State = serde_json::from_value(value.clone())
        .map_err(|_| DeploymentError::ReconciliationRequired)?;
    if state.version <= 0
        || state.generation <= 0
        || serde_json::to_value(&state).map_err(|_| DeploymentError::Serialization)? != *value
    {
        return Err(DeploymentError::ReconciliationRequired);
    }
    Ok(state)
}

pub(super) fn context(value: &Value, bound: usize) -> Result<(), DeploymentError> {
    let object = value
        .as_object()
        .ok_or(DeploymentError::ReconciliationRequired)?;
    if object.keys().any(|key| {
        !matches!(
            key.as_str(),
            "claims" | "closures" | "pending" | "recovery" | "detach" | "terminal"
        )
    }) {
        return Err(DeploymentError::ReconciliationRequired);
    }
    array(value, "claims", bound)?;
    array(value, "closures", bound)?;
    optional(value, "pending", bound)?;
    optional(value, "recovery", bound)?;
    optional(value, "detach", bound)?;
    optional(value, "terminal", bound)?;
    Ok(())
}
