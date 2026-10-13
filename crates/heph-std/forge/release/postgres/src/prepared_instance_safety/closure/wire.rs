//! Strict native142 comparison data; neither decoding nor construction grants IO.
use super::writing;
use identity_domain::{RequestId, UserId};
use recipe_application::{
    CommandIdentity, DeploymentAttemptId, DeploymentCommandId, DeploymentError,
    DeploymentOperation, DiagnosticCode, EffectClaim, InstallProgress,
    InstanceClosurePublicationOrigin, InstanceClosurePublicationProgress,
    PreparedInstanceSafetyTarget, QualifiedFreshInstanceClosurePublication, RemovalProgress,
    ResourceAction, ResourceProgress, ResumedPreparedInstanceClosureRequest,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

pub(super) fn fields(value: &Value, names: &[&str]) -> Result<(), DeploymentError> {
    if value.as_object().is_none_or(|object| {
        object.len() != names.len() || names.iter().any(|name| !object.contains_key(*name))
    }) {
        return Err(DeploymentError::ReconciliationRequired);
    }
    Ok(())
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct State {
    install: InstallProgress,
    removal: RemovalProgress,
    version: i64,
    active_attempt: Option<Uuid>,
    diagnostic: Option<DiagnosticCode>,
    generation: i64,
    last_action: Option<ResourceAction>,
}

pub(super) fn progress(
    value: &Value,
    drain: &EffectClaim,
) -> Result<InstanceClosurePublicationProgress, DeploymentError> {
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
    if serde_json::to_value(&state).map_err(|_| DeploymentError::Serialization)? != *value {
        return Err(DeploymentError::ReconciliationRequired);
    }
    InstanceClosurePublicationProgress::new(
        drain.clone(),
        ResourceProgress {
            install: state.install,
            removal: state.removal,
            version: u64::try_from(state.version).map_err(|_| DeploymentError::IntentMismatch)?,
            active_attempt: state
                .active_attempt
                .map(DeploymentAttemptId::from_uuid)
                .transpose()?,
            diagnostic: state.diagnostic,
        },
        u64::try_from(state.generation).map_err(|_| DeploymentError::IntentMismatch)?,
        state.last_action,
    )
}

pub(super) fn pending(
    value: &Value,
    target: &PreparedInstanceSafetyTarget,
    drain: &EffectClaim,
) -> Result<InstanceClosurePublicationProgress, DeploymentError> {
    fields(
        value,
        &[
            "resource",
            "drain_claim",
            "actor",
            "idempotency",
            "create_claim",
            "create_actor",
            "create_idempotency",
            "prepared",
            "observed_progress",
        ],
    )?;
    let create = target.original();
    if value["resource"].as_str() != Some(drain.resource.as_str())
        || value["drain_claim"] != writing::claim(drain)?
        || value["actor"]
            != serde_json::to_value(drain.command.actor_id())
                .map_err(|_| DeploymentError::Serialization)?
        || value["idempotency"]
            != serde_json::to_value(drain.command.idempotency_id())
                .map_err(|_| DeploymentError::Serialization)?
        || value["create_claim"] != writing::claim(create)?
        || value["create_actor"]
            != serde_json::to_value(create.command.actor_id())
                .map_err(|_| DeploymentError::Serialization)?
        || value["create_idempotency"]
            != serde_json::to_value(create.command.idempotency_id())
                .map_err(|_| DeploymentError::Serialization)?
        || value["prepared"]
            != serde_json::to_value(target.prepared())
                .map_err(|_| DeploymentError::Serialization)?
    {
        return Err(DeploymentError::IntentMismatch);
    }
    progress(&value["observed_progress"], drain)
}

pub(super) fn admitted_pending<'a>(
    context: &'a Value,
    version: i64,
    request: &ResumedPreparedInstanceClosureRequest,
) -> Result<&'a Value, DeploymentError> {
    let object = context
        .as_object()
        .ok_or(DeploymentError::ReconciliationRequired)?;
    if object
        .keys()
        .any(|key| !matches!(key.as_str(), "claims" | "closures" | "pending" | "recovery"))
        || u64::try_from(version).ok() != Some(request.expected_deployment_version())
    {
        return Err(DeploymentError::IntentMismatch);
    }
    let bound = request.intent().resources().len();
    for name in ["claims", "closures", "pending", "recovery"] {
        let Some(value) = object.get(name) else {
            if matches!(name, "claims" | "closures" | "pending") {
                return Err(DeploymentError::ReconciliationRequired);
            }
            continue;
        };
        let entries = value
            .as_array()
            .ok_or(DeploymentError::ReconciliationRequired)?;
        if entries.len() > bound || (matches!(name, "pending" | "recovery") && entries.is_empty()) {
            return Err(DeploymentError::ReconciliationRequired);
        }
    }
    let entries = context["pending"]
        .as_array()
        .ok_or(DeploymentError::ReconciliationRequired)?;
    let mut matching = entries
        .iter()
        .filter(|entry| entry["resource"].as_str() == Some(request.drain().resource.as_str()));
    let value = matching
        .next()
        .ok_or(DeploymentError::ReconciliationRequired)?;
    if matching.next().is_some() {
        return Err(DeploymentError::ReconciliationRequired);
    }
    let original = pending(value, request.target(), request.drain())?;
    if request
        .pending()
        .snapshot()
        .resources
        .get(&request.drain().resource)
        != Some(original.progress())
    {
        return Err(DeploymentError::IntentMismatch);
    }
    // This is the unchanged full database JSON, not an invented generation or a
    // historical snapshot rebuilt from today's resource state. Native142 repeats
    // complete admission/original/config/current-prefix qualification under locks.
    Ok(value)
}

fn uuid(value: &Value, name: &str) -> Result<Uuid, DeploymentError> {
    let id = value[name]
        .as_str()
        .ok_or(DeploymentError::ReconciliationRequired)?
        .parse::<Uuid>()
        .map_err(|_| DeploymentError::ReconciliationRequired)?;
    if id.is_nil() {
        return Err(DeploymentError::ReconciliationRequired);
    }
    Ok(id)
}

pub fn publication(
    value: Option<&Value>,
    target: &PreparedInstanceSafetyTarget,
    drain: &EffectClaim,
) -> Result<InstanceClosurePublicationOrigin, DeploymentError> {
    let Some(value) = value else {
        return Ok(InstanceClosurePublicationOrigin::OriginalDrain);
    };
    fields(
        value,
        &[
            "kind",
            "command_id",
            "actor",
            "idempotency",
            "admission_request",
            "publication_request",
            "admission_version",
            "pending",
        ],
    )?;
    if value["kind"] != "qualified_fresh_remove" {
        return Err(DeploymentError::ReconciliationRequired);
    }
    let observed = pending(&value["pending"], target, drain)?;
    let command = CommandIdentity::from_recorded(
        UserId::from_uuid(uuid(value, "actor")?),
        RequestId::from_uuid(uuid(value, "idempotency")?),
        DeploymentOperation::Remove,
    )?;
    Ok(InstanceClosurePublicationOrigin::QualifiedFreshRemove(
        Box::new(QualifiedFreshInstanceClosurePublication::new(
            drain.clone(),
            command,
            DeploymentCommandId::from_uuid(uuid(value, "command_id")?)?,
            RequestId::from_uuid(uuid(value, "admission_request")?),
            RequestId::from_uuid(uuid(value, "publication_request")?),
            value["admission_version"]
                .as_u64()
                .ok_or(DeploymentError::ReconciliationRequired)?,
            observed,
        )?),
    ))
}
