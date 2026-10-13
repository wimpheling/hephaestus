//! Strict initial wire decoding; native qualification remains independently mandatory.
use super::super::{origin::fields, wire};
use crate::{decode_hash, prepared_instance_safety};
use identity_domain::{RequestId, UserId};
use recipe_application::{
    AttemptProvenance, CommandIdentity, DeploymentAttemptId, DeploymentError, DeploymentIntent,
    DeploymentOperation, DiagnosticCode, EffectClaim, InstallProgress, OriginalInstanceClosure,
    PreparedInstanceTerminalPreparationInput, RemovalProgress, ResourceAction,
};
use release_domain::{ContentHash, InstanceRemovalId, ReleaseCommandKey};
use release_service::InstanceExecutionConfiguration;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use uuid::Uuid;

pub(super) fn selected<'a>(
    context: &'a Value,
    intent: &DeploymentIntent,
    resource: &str,
) -> Result<&'a Value, DeploymentError> {
    let object = context
        .as_object()
        .ok_or(DeploymentError::ReconciliationRequired)?;
    if !object.contains_key("claims")
        || !object.contains_key("closures")
        || object.keys().any(|key| {
            !matches!(
                key.as_str(),
                "claims" | "closures" | "pending" | "recovery" | "detach" | "terminal"
            )
        })
    {
        return Err(DeploymentError::ReconciliationRequired);
    }
    for (name, array) in object {
        let entries = array
            .as_array()
            .ok_or(DeploymentError::ReconciliationRequired)?;
        if entries.len() > intent.resources().len()
            || (!matches!(name.as_str(), "claims" | "closures") && entries.is_empty())
        {
            return Err(DeploymentError::ReconciliationRequired);
        }
        let mut seen = BTreeSet::new();
        for entry in entries {
            let name = if name == "claims" {
                entry["claim"]["resource"].as_str()
            } else {
                entry["resource"].as_str()
            }
            .ok_or(DeploymentError::ReconciliationRequired)?;
            if !seen.insert(name) || !intent.resources().keys().any(|key| key.as_str() == name) {
                return Err(DeploymentError::IntentMismatch);
            }
        }
    }
    let matches: Vec<_> = context["closures"]
        .as_array()
        .ok_or(DeploymentError::ReconciliationRequired)?
        .iter()
        .filter(|entry| entry["resource"].as_str() == Some(resource))
        .collect();
    let [value] = matches.as_slice() else {
        return Err(DeploymentError::ReconciliationRequired);
    };
    Ok(*value)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Claim {
    command_id: Uuid,
    deployment_id: Uuid,
    resource: String,
    identity: Value,
    input_hash: ContentHash,
    resource_version: u64,
    generation: u64,
    action: ResourceAction,
    attempt_id: Uuid,
    actor_id: Uuid,
    request_id: Uuid,
}
pub(super) fn closure(
    native: &Value,
    input: &PreparedInstanceTerminalPreparationInput,
) -> Result<OriginalInstanceClosure, DeploymentError> {
    let stored: Claim = serde_json::from_value(native["drain_claim"].clone())
        .map_err(|_| DeploymentError::ReconciliationRequired)?;
    let original = input.target().original();
    let command = CommandIdentity::from_recorded(
        UserId::from_uuid(uuid(native, "actor")?),
        RequestId::from_uuid(uuid(native, "idempotency")?),
        DeploymentOperation::Remove,
    )?;
    if stored.command_id != command.id().as_uuid()
        || stored.actor_id != command.actor_id().as_uuid()
        || stored.deployment_id != original.deployment_id.as_uuid()
        || stored.resource != original.resource.as_str()
        || stored.identity
            != serde_json::to_value(original.identity)
                .map_err(|_| DeploymentError::Serialization)?
        || stored.input_hash != original.input_hash
        || stored.action != ResourceAction::Drain
        || stored.request_id.is_nil()
        || stored.generation == 0
        || stored.generation > i64::MAX.cast_unsigned()
        || stored.resource_version == 0
        || stored.resource_version > i64::MAX.cast_unsigned()
    {
        return Err(DeploymentError::IntentMismatch);
    }
    let drain = EffectClaim {
        command,
        deployment_id: original.deployment_id,
        resource: original.resource.clone(),
        identity: original.identity,
        input_hash: stored.input_hash,
        resource_version: stored.resource_version,
        generation: stored.generation,
        action: stored.action,
        provenance: AttemptProvenance {
            attempt_id: DeploymentAttemptId::from_uuid(stored.attempt_id)?,
            actor_id: UserId::from_uuid(stored.actor_id),
            request_id: RequestId::from_uuid(stored.request_id),
        },
    };
    if native["drain_claim"] != prepared_instance_safety::claim(&drain)? {
        return Err(DeploymentError::IntentMismatch);
    }
    let removal = &native["removal"];
    let result = OriginalInstanceClosure {
        publication_origin: prepared_instance_safety::publication(
            native.get("publication"),
            input.target(),
            &drain,
        )?,
        drain,
        removal_id: InstanceRemovalId::from_uuid(uuid(removal, "id")?),
        command_key: serde_json::from_value::<ReleaseCommandKey>(json!(bytea(
            &native["inbox"],
            "command_key"
        )?))
        .map_err(|_| DeploymentError::Serialization)?,
        expected_version: positive_version(&removal["expected_version"])?,
        actor: UserId::from_uuid(uuid(removal, "created_by")?),
        request: RequestId::from_uuid(uuid(removal, "request_id")?),
        input_hash: ContentHash::from_digest(bytea(removal, "input_hash")?),
    };
    result.validate(input.effect())?;
    Ok(result)
}
fn bytea(value: &Value, name: &str) -> Result<[u8; 32], DeploymentError> {
    let hex = value[name]
        .as_str()
        .and_then(|value| value.strip_prefix("\\x"))
        .ok_or(DeploymentError::ReconciliationRequired)?;
    decode_hash(hex).map_err(|_| DeploymentError::ReconciliationRequired)
}
fn uuid(value: &Value, name: &str) -> Result<Uuid, DeploymentError> {
    value[name]
        .as_str()
        .and_then(|value| value.parse::<Uuid>().ok())
        .filter(|id| !id.is_nil())
        .ok_or(DeploymentError::ReconciliationRequired)
}
fn positive_version(value: &Value) -> Result<u64, DeploymentError> {
    value
        .as_u64()
        .filter(|value| *value > 0 && *value < i64::MAX.cast_unsigned())
        .ok_or(DeploymentError::StaleClaim)
}

pub(super) fn transport(
    input: &PreparedInstanceTerminalPreparationInput,
    configuration: &InstanceExecutionConfiguration,
    closure: &Value,
) -> Result<Value, DeploymentError> {
    wire::configured(configuration, input.target(), input.configuration())?;
    let command = input.current_command();
    Ok(
        json!({"version":1,"project":input.intent().project_id(),"deployment":input.intent().id(),
        "intent_hash":input.intent().input_hash(),"resource":input.effect().resource,
        "create_claim":prepared_instance_safety::claim(input.target().original())?,"prepared":input.target().prepared(),
        "closure":closure,"delete_or_detach_claim":prepared_instance_safety::claim(input.effect())?,
        "current_command":{"id":command.id(),"actor_id":command.actor_id(),"idempotency_id":command.idempotency_id(),"operation":command.operation()},
        "configuration":configuration.canonical_json(),"before_instance_version":null}),
    )
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ObservedProgress {
    install: InstallProgress,
    removal: RemovalProgress,
    version: u64,
    active_attempt: Option<Uuid>,
    diagnostic: Option<DiagnosticCode>,
    generation: u64,
    last_action: Option<ResourceAction>,
}
fn observed(value: &Value) -> Result<(), DeploymentError> {
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
    let state: ObservedProgress = serde_json::from_value(value.clone())
        .map_err(|_| DeploymentError::ReconciliationRequired)?;
    if state.version == 0
        || state.version > i64::MAX.cast_unsigned()
        || state.generation == 0
        || state.generation > i64::MAX.cast_unsigned()
        || state.active_attempt.is_some_and(|id| id.is_nil())
        || serde_json::to_value(state).map_err(|_| DeploymentError::Serialization)? != *value
    {
        return Err(DeploymentError::ReconciliationRequired);
    }
    Ok(())
}
pub(super) fn continuation(value: &Value, transport: &Value) -> Result<u64, DeploymentError> {
    fields(value, &["version", "request", "history"])?;
    let version = positive_version(&value["request"]["before_instance_version"])?;
    let mut expected = transport.clone();
    expected["before_instance_version"] = json!(version);
    if value["version"] != 1 || value["request"] != expected {
        return Err(DeploymentError::IntentMismatch);
    }
    let history = &value["history"];
    fields(
        history,
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
    observed(&history["observed_progress"])?;
    if history["resource"] != expected["resource"]
        || history["delete_claim"] != expected["delete_or_detach_claim"]
        || history["before_instance_version"] != json!(version)
    {
        return Err(DeploymentError::IntentMismatch);
    }
    if history["terminal_operation_id"].is_null() {
        if !history["after_instance_version"].is_null() || !history["publisher"].is_null() {
            return Err(DeploymentError::ReconciliationRequired);
        }
    } else {
        uuid(history, "terminal_operation_id")?;
        if history["after_instance_version"].as_u64() != version.checked_add(1) {
            return Err(DeploymentError::IntentMismatch);
        }
        let publisher = &history["publisher"];
        fields(
            publisher,
            &[
                "command_id",
                "actor_id",
                "request_id",
                "admission_command_id",
            ],
        )?;
        for name in ["command_id", "actor_id", "request_id"] {
            uuid(publisher, name)?;
        }
        if !publisher["admission_command_id"].is_null() {
            uuid(publisher, "admission_command_id")?;
        }
    }
    Ok(version)
}
