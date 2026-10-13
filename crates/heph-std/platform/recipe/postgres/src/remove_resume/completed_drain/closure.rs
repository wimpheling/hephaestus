//! Full native closure comparison; recorded publishers are never credentials.
// The accepted context decoder's publication module is private. This bounded
// target-bound comparison uses the existing ClaimWire and complete State decoder;
// it neither copies context inventories nor reconstructs a historical snapshot.
use super::decode_state;
use crate::execution_rows::ClaimWire;
use recipe_application::{
    DeploymentError, ExistingInstanceClosureRequest, InstanceClosurePublicationOrigin,
    InstanceClosurePublicationProgress, OriginalInstanceClosure, PreparedInstanceSafetyTarget,
};
use serde_json::{Value, json};

pub(super) fn compare(
    actual: &Value,
    request: &ExistingInstanceClosureRequest,
) -> Result<(), DeploymentError> {
    let closure = request.expected();
    let mut expected = json!({
        "drain": ClaimWire::new(&closure.drain)?,
        "actor": closure.drain.command.actor_id(),
        "idempotency": closure.drain.command.idempotency_id(),
        "removal_id": closure.removal_id, "command_key": closure.command_key,
        "expected_version": closure.expected_version, "creator": closure.actor,
        "request": closure.request, "input_hash": closure.input_hash,
    });
    if let Some(publication) = publication(actual.get("publication"), closure, request.target())? {
        expected["publication"] = publication;
    }
    if actual != &expected {
        return Err(DeploymentError::ReconciliationRequired);
    }
    Ok(())
}

fn publication(
    actual: Option<&Value>,
    closure: &OriginalInstanceClosure,
    target: &PreparedInstanceSafetyTarget,
) -> Result<Option<Value>, DeploymentError> {
    match (&closure.publication_origin, actual) {
        (InstanceClosurePublicationOrigin::OriginalDrain, None) => Ok(None),
        (InstanceClosurePublicationOrigin::QualifiedFreshRemove(publisher), Some(actual)) => {
            fields(
                actual,
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
            let pending = &actual["pending"];
            compare_pending(pending, closure, target, publisher.pending())?;
            // Keep every original native Pending field/value; only after all its
            // Create/Drain/preparation/progress fields have been compared.
            Ok(Some(
                json!({"kind":"qualified_fresh_remove","command_id":publisher.native_command_id(),
                "actor":publisher.command().actor_id(),"idempotency":publisher.command().idempotency_id(),
                "admission_request":publisher.admission_request(),"publication_request":publisher.publication_request(),
                "admission_version":publisher.admission_version(),"pending":pending}),
            ))
        }
        // Explicit null/unknown tags and any missing fresh tag are never retried
        // as OriginalDrain. Saved B remains independent of current manager C.
        _ => Err(DeploymentError::ReconciliationRequired),
    }
}

fn fields(value: &Value, names: &[&str]) -> Result<(), DeploymentError> {
    if value.as_object().is_none_or(|object| {
        object.len() != names.len() || names.iter().any(|name| !object.contains_key(*name))
    }) {
        return Err(DeploymentError::ReconciliationRequired);
    }
    Ok(())
}

fn compare_pending(
    actual: &Value,
    closure: &OriginalInstanceClosure,
    target: &PreparedInstanceSafetyTarget,
    expected: &InstanceClosurePublicationProgress,
) -> Result<(), DeploymentError> {
    fields(
        actual,
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
    let drain = &closure.drain;
    let create = target.original();
    let mut roots = json!({"resource":drain.resource,"drain_claim":ClaimWire::new(drain)?,
        "actor":drain.command.actor_id(),"idempotency":drain.command.idempotency_id(),
        "create_claim":ClaimWire::new(create)?,"create_actor":create.command.actor_id(),
        "create_idempotency":create.command.idempotency_id(),"prepared":target.prepared()});
    let state = decode_state(&actual["observed_progress"])?;
    let observed = InstanceClosurePublicationProgress::new(
        drain.clone(),
        state.progress()?,
        u64::try_from(state.generation).map_err(|_| DeploymentError::IntentMismatch)?,
        state.last_action,
    )?;
    if observed != *expected {
        return Err(DeploymentError::IntentMismatch);
    }
    roots["observed_progress"] =
        serde_json::to_value(&state).map_err(|_| DeploymentError::Serialization)?;
    if actual != &roots {
        return Err(DeploymentError::IntentMismatch);
    }
    Ok(())
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
