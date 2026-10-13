//! Original Create/Drain and full per-resource native state comparison.
use super::{Roots, original, wire};
use crate::effect_state::State;
use recipe_application::{
    DeploymentError, EffectClaim, InstanceClosurePublicationProgress, PreparedInstanceSafetyTarget,
    ResourceAction,
};
use serde_json::Value;

pub(super) struct Pending {
    pub source: Value,
    pub target: PreparedInstanceSafetyTarget,
    pub drain: EffectClaim,
    pub state: State,
}

pub(super) fn decode(value: &Value, roots: &Roots<'_>) -> Result<Pending, DeploymentError> {
    wire::fields(
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
    let drain = original(value, "drain_claim", roots.intent)?;
    if value.get("resource").and_then(Value::as_str) != Some(drain.resource.as_str()) {
        return Err(DeploymentError::IntentMismatch);
    }
    let create_data = serde_json::json!({"claim":value["create_claim"],
        "actor":value["create_actor"],"idempotency":value["create_idempotency"]});
    let create = super::claim(
        &create_data,
        "claim",
        roots.intent,
        recipe_application::DeploymentOperation::Install,
    )?;
    let prepared = roots
        .preparation
        .instances()
        .iter()
        .find(|item| item.resource() == &drain.resource)
        .ok_or(DeploymentError::IntentMismatch)?;
    if create.command != roots.preparation.command()
        || create.action != ResourceAction::Create
        || create.resource != drain.resource
        || create.identity != drain.identity
        || create.input_hash != drain.input_hash
        || serde_json::to_value(prepared).map_err(|_| DeploymentError::Serialization)?
            != value["prepared"]
    {
        return Err(DeploymentError::IntentMismatch);
    }
    let target = PreparedInstanceSafetyTarget::new(
        create,
        prepared.clone(),
        roots.config.namespace().to_owned(),
        roots.config.host().to_owned(),
    )?;
    let state = wire::state(&value["observed_progress"])?;
    progress(&drain, &state)?;
    Ok(Pending {
        source: value.clone(),
        target,
        drain,
        state,
    })
}

pub(super) fn progress(
    drain: &EffectClaim,
    state: &State,
) -> Result<InstanceClosurePublicationProgress, DeploymentError> {
    InstanceClosurePublicationProgress::new(
        drain.clone(),
        state.progress()?,
        crate::hydration::unsigned(state.generation)?,
        state.last_action,
    )
}
