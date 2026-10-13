//! Exact native143 comparison wire; no lookup or absence is inferred here.
use super::super::wire;
use crate::{effect_state::State, execution_rows::ClaimWire};
use recipe_application::{DeploymentError, ResourceAction};
use serde_json::Value;
use uuid::Uuid;

#[derive(Clone)]
pub(in crate::remove_resume) struct Entry {
    pub resource: capability_domain::CapabilitySlotKey,
    pub claim: ClaimWire,
    pub state: State,
    pub before: Option<u64>,
    pub operation: Option<Uuid>,
    pub after: Option<u64>,
    pub publisher: Option<Value>,
}

pub(super) fn entries(
    data: &Value,
    field: &str,
    bound: usize,
) -> Result<Vec<Entry>, DeploymentError> {
    let mut names = std::collections::BTreeSet::new();
    wire::optional(data, field, bound)?
        .iter()
        .map(|value| {
            let deleting = field == "terminal";
            let (claim_field, action) = if deleting {
                wire::fields(
                    value,
                    &[
                        "resource",
                        "delete_claim",
                        "observed_progress",
                        "before_instance_version",
                        "terminal_operation_id",
                        "after_instance_version",
                        "publisher",
                    ],
                )?;
                ("delete_claim", ResourceAction::Delete)
            } else {
                wire::fields(value, &["resource", "detach_claim", "observed_progress"])?;
                ("detach_claim", ResourceAction::Detach)
            };
            let resource = capability_domain::CapabilitySlotKey::parse(
                value["resource"]
                    .as_str()
                    .ok_or(DeploymentError::IntentMismatch)?,
            )
            .map_err(|_| DeploymentError::IntentMismatch)?;
            let claim: ClaimWire = serde_json::from_value(value[claim_field].clone())
                .map_err(|_| DeploymentError::ReconciliationRequired)?;
            if !names.insert(resource.clone())
                || claim.resource != resource.as_str()
                || claim.action != action
                || claim.attempt_id.is_nil()
                || claim.command_id.is_nil()
                || claim.deployment_id.is_nil()
                || claim.actor_id.is_nil()
                || claim.request_id.is_nil()
                || claim.resource_version == 0
                || claim.resource_version > i64::MAX.cast_unsigned()
                || claim.generation == 0
                || claim.generation > i64::MAX.cast_unsigned()
                || serde_json::to_value(&claim).map_err(|_| DeploymentError::Serialization)?
                    != value[claim_field]
            {
                return Err(DeploymentError::IntentMismatch);
            }
            let state = wire::state(&value["observed_progress"])?;
            let mut entry = Entry {
                resource,
                claim,
                state,
                before: None,
                operation: None,
                after: None,
                publisher: None,
            };
            if deleting {
                entry.before = Some(positive(&value["before_instance_version"])?);
                if entry.before == Some(i64::MAX.cast_unsigned()) {
                    return Err(DeploymentError::StaleClaim);
                }
                let present = [
                    "terminal_operation_id",
                    "after_instance_version",
                    "publisher",
                ]
                .map(|name| !value[name].is_null());
                match present {
                    [false, false, false] => {
                        if entry.state.active_attempt != Some(entry.claim.attempt_id) {
                            return Err(DeploymentError::ReconciliationRequired);
                        }
                    }
                    [true, true, true] => {
                        entry.operation = Some(wire::uuid(value, "terminal_operation_id")?);
                        entry.after = Some(positive(&value["after_instance_version"])?);
                        if entry.before.and_then(|before| before.checked_add(1)) != entry.after {
                            return Err(DeploymentError::StaleClaim);
                        }
                        publisher(&value["publisher"])?;
                        entry.publisher = Some(value["publisher"].clone());
                    }
                    _ => return Err(DeploymentError::ReconciliationRequired),
                }
            }
            validate_state(&entry, deleting)?;
            Ok(entry)
        })
        .collect()
}

pub(super) fn publisher(value: &Value) -> Result<(), DeploymentError> {
    wire::fields(
        value,
        &[
            "command_id",
            "actor_id",
            "request_id",
            "admission_command_id",
        ],
    )?;
    for name in ["command_id", "actor_id", "request_id"] {
        wire::uuid(value, name)?;
    }
    if !value["admission_command_id"].is_null() {
        wire::uuid(value, "admission_command_id")?;
    }
    Ok(())
}
fn positive(value: &Value) -> Result<u64, DeploymentError> {
    value
        .as_u64()
        .filter(|value| *value > 0 && *value <= i64::MAX.cast_unsigned())
        .ok_or(DeploymentError::StaleClaim)
}

// Bind every raw State slot to the exact original attempt before row hydration.
// This checks comparison shape only; protected native history remains required.
fn validate_state(entry: &Entry, deleting: bool) -> Result<(), DeploymentError> {
    use recipe_application::{InstallProgress, RemovalProgress};
    let observed = &entry.state;
    let version = u64::try_from(observed.version).map_err(|_| DeploymentError::IntentMismatch)?;
    if observed.install != InstallProgress::Ready
        || u64::try_from(observed.generation).ok() != Some(entry.claim.generation)
        || version < entry.claim.resource_version
    {
        return Err(DeploymentError::IntentMismatch);
    }
    let valid = match observed.active_attempt {
        Some(attempt) if attempt == entry.claim.attempt_id => {
            let prior = if deleting {
                ResourceAction::Detach
            } else {
                ResourceAction::Drain
            };
            observed.last_action == Some(prior)
                && if deleting && version == entry.claim.resource_version {
                    observed.removal == RemovalProgress::Detached && observed.diagnostic.is_none()
                } else {
                    version > entry.claim.resource_version
                        && observed.removal == RemovalProgress::RecoveryRequired
                        && observed.diagnostic.is_some()
                }
        }
        None if deleting && entry.operation.is_some() => {
            version > entry.claim.resource_version
                && observed.removal == RemovalProgress::Deleted
                && observed.last_action == Some(ResourceAction::Delete)
                && observed.diagnostic.is_none()
        }
        _ => false,
    };
    if !valid {
        return Err(DeploymentError::IntentMismatch);
    }
    Ok(())
}
