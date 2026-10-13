//! Explicit publication tag; recorded publisher is never current credentials.
use super::{Roots, pending, wire};
use identity_domain::RequestId;
use recipe_application::{
    DeploymentCommandId, DeploymentError, DeploymentOperation, EffectClaim,
    InstanceClosurePublicationOrigin, QualifiedFreshInstanceClosurePublication,
};
use serde_json::{Value, json};

pub(super) fn decode(
    value: Option<&Value>,
    drain: &EffectClaim,
    roots: Option<&Roots<'_>>,
) -> Result<InstanceClosurePublicationOrigin, DeploymentError> {
    let Some(value) = value else {
        return Ok(InstanceClosurePublicationOrigin::OriginalDrain);
    };
    let roots = roots.ok_or(DeploymentError::ReconciliationRequired)?;
    wire::fields(
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
    let original = pending::decode(&value["pending"], roots)?;
    if original.drain != *drain {
        return Err(DeploymentError::IntentMismatch);
    }
    let command = wire::command(value, DeploymentOperation::Remove)?;
    let publication = QualifiedFreshInstanceClosurePublication::new(
        drain.clone(),
        command,
        DeploymentCommandId::from_uuid(wire::uuid(value, "command_id")?)?,
        RequestId::from_uuid(wire::uuid(value, "admission_request")?),
        RequestId::from_uuid(wire::uuid(value, "publication_request")?),
        value["admission_version"]
            .as_u64()
            .ok_or(DeploymentError::ReconciliationRequired)?,
        pending::progress(drain, &original.state)?,
    )?;
    Ok(InstanceClosurePublicationOrigin::QualifiedFreshRemove(
        Box::new(publication),
    ))
}

pub(super) fn encode(
    origin: &InstanceClosurePublicationOrigin,
    roots: &Roots<'_>,
    source: &Value,
) -> Result<Option<Value>, DeploymentError> {
    match origin {
        InstanceClosurePublicationOrigin::OriginalDrain => Ok(None),
        InstanceClosurePublicationOrigin::QualifiedFreshRemove(publication) => {
            // Retain ALL nine native Pending fields, including original Create data.
            // The Core tag deliberately stores only per-resource comparison progress.
            let pending = source
                .get("publication")
                .and_then(|value| value.get("pending"))
                .ok_or(DeploymentError::ReconciliationRequired)?;
            let decoded = pending::decode(pending, roots)?;
            if pending::progress(&decoded.drain, &decoded.state)? != *publication.pending() {
                return Err(DeploymentError::IntentMismatch);
            }
            Ok(Some(
                json!({"kind":"qualified_fresh_remove","command_id":publication.native_command_id(),
                "actor":publication.command().actor_id(),"idempotency":publication.command().idempotency_id(),
                "admission_request":publication.admission_request(),"publication_request":publication.publication_request(),
                "admission_version":publication.admission_version(),"pending":pending}),
            ))
        }
    }
}
