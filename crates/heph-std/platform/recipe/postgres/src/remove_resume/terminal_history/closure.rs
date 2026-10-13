//! Actual native closure rows; recorded publisher never authenticates a reader.
use crate::execution_rows::ClaimWire;
use recipe_application::{
    DeploymentError, InstanceClosurePublicationOrigin, PreparedInstanceDeleteRequest,
};
use serde_json::{Value, json};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

pub(super) async fn worker(
    tx: &mut Transaction<'_, Postgres>,
    request: &PreparedInstanceDeleteRequest,
) -> Result<Value, DeploymentError> {
    let instance = request.target().prepared().instance_id();
    let rows: Vec<(Value,Value,Option<Uuid>)> = sqlx::query_as("SELECT to_jsonb(removal)-'prepared_resume_command_id',to_jsonb(inbox),removal.prepared_resume_command_id FROM agent_instance_removal_requests removal JOIN release_command_inbox inbox ON inbox.secondary_id=removal.id OR inbox.command_key=$3 WHERE removal.id=$1 AND removal.instance_id=$2")
        .bind(request.closure().removal_id.as_uuid()).bind(instance.as_uuid())
        .bind(request.closure().command_key.as_bytes()).fetch_all(&mut **tx).await.map_err(super::storage)?;
    if rows.len() != 1 {
        return Err(DeploymentError::ReconciliationRequired);
    }
    let (removal, inbox, link) = &rows[0];
    let drain = &request.closure().drain;
    let mut result = json!({"resource":drain.resource,"drain_claim":ClaimWire::new(drain)?,
        "actor":drain.command.actor_id(),"idempotency":drain.command.idempotency_id(),"removal":removal,"inbox":inbox});
    match &request.closure().publication_origin {
        InstanceClosurePublicationOrigin::OriginalDrain if link.is_none() => {}
        InstanceClosurePublicationOrigin::QualifiedFreshRemove(saved)
            if *link == Some(saved.native_command_id().as_uuid()) =>
        {
            let (version, actor, admission_request, context):(i64,Uuid,Uuid,Value)=sqlx::query_as("SELECT expected_version,actor_id,request_id,context_json FROM recipe_remove_resume_admissions WHERE command_id=$1")
                .bind(saved.native_command_id().as_uuid()).fetch_one(&mut **tx).await.map_err(super::storage)?;
            let pending = context["pending"]
                .as_array()
                .ok_or(DeploymentError::ReconciliationRequired)?;
            let matches: Vec<_> = pending
                .iter()
                .filter(|item| item["resource"] == drain.resource.as_str())
                .collect();
            if matches.len() != 1
                || u64::try_from(version).ok() != Some(saved.admission_version())
                || actor != saved.command().actor_id().as_uuid()
                || admission_request != saved.admission_request().as_uuid()
            {
                return Err(DeploymentError::IntentMismatch);
            }
            result["publication"] = json!({"kind":"qualified_fresh_remove","command_id":saved.native_command_id(),
                "actor":saved.command().actor_id(),"idempotency":saved.command().idempotency_id(),
                "admission_request":saved.admission_request(),"publication_request":saved.publication_request(),
                "admission_version":saved.admission_version(),"pending":matches[0]});
        }
        _ => return Err(DeploymentError::IntentMismatch),
    }
    compare(tx, &result, request).await?;
    Ok(result)
}

pub(super) async fn compare(
    tx: &mut Transaction<'_, Postgres>,
    actual: &Value,
    request: &PreparedInstanceDeleteRequest,
) -> Result<(), DeploymentError> {
    // Extract only represented Core pins for comparison. The FULL unchanged raw
    // object goes to native143, which independently validates all remaining fields.
    let (id,key,version,actor,publication_request,hash):(Uuid,Vec<u8>,i64,Uuid,Uuid,Vec<u8>)=sqlx::query_as("SELECT ($1->'removal'->>'id')::uuid,decode(substr($1->'inbox'->>'command_key',3),'hex'),($1->'removal'->>'expected_version')::bigint,($1->'removal'->>'created_by')::uuid,($1->'removal'->>'request_id')::uuid,decode(substr($1->'removal'->>'input_hash',3),'hex')")
        .bind(actual).fetch_one(&mut **tx).await.map_err(super::storage)?;
    let closure = request.closure();
    let mut fields = vec![
        "resource",
        "drain_claim",
        "actor",
        "idempotency",
        "removal",
        "inbox",
    ];
    if actual.get("publication").is_some() {
        fields.push("publication");
    }
    fields_match(actual, &fields)?;
    if actual["resource"] != closure.drain.resource.as_str()
        || actual["drain_claim"]
            != serde_json::to_value(ClaimWire::new(&closure.drain)?)
                .map_err(|_| DeploymentError::Serialization)?
        || actual["actor"] != json!(closure.drain.command.actor_id())
        || actual["idempotency"] != json!(closure.drain.command.idempotency_id())
        || id != closure.removal_id.as_uuid()
        || key.as_slice() != closure.command_key.as_bytes()
        || u64::try_from(version).ok() != Some(closure.expected_version)
        || actor != closure.actor.as_uuid()
        || publication_request != closure.request.as_uuid()
        || hash.as_slice() != closure.input_hash.as_bytes()
    {
        return Err(DeploymentError::IntentMismatch);
    }
    publication(actual.get("publication"), request)
}
fn fields_match(value: &Value, fields: &[&str]) -> Result<(), DeploymentError> {
    if value.as_object().is_none_or(|object| {
        object.len() != fields.len() || fields.iter().any(|name| !object.contains_key(*name))
    }) {
        return Err(DeploymentError::ReconciliationRequired);
    }
    Ok(())
}
fn publication(
    actual: Option<&Value>,
    request: &PreparedInstanceDeleteRequest,
) -> Result<(), DeploymentError> {
    match (&request.closure().publication_origin, actual) {
        (InstanceClosurePublicationOrigin::OriginalDrain, None) => Ok(()),
        (InstanceClosurePublicationOrigin::QualifiedFreshRemove(saved), Some(actual)) => {
            let progress = saved.pending();
            let state = json!({"install":progress.progress().install,"removal":progress.progress().removal,
                "version":progress.progress().version,"active_attempt":progress.progress().active_attempt,
                "diagnostic":progress.progress().diagnostic,"generation":progress.generation(),"last_action":progress.last_action()});
            let target = request.target();
            let create = target.original();
            let drain = &request.closure().drain;
            let expected = json!({"kind":"qualified_fresh_remove","command_id":saved.native_command_id(),"actor":saved.command().actor_id(),
                "idempotency":saved.command().idempotency_id(),"admission_request":saved.admission_request(),
                "publication_request":saved.publication_request(),"admission_version":saved.admission_version(),
                "pending":{"resource":drain.resource,"drain_claim":ClaimWire::new(drain)?,"actor":drain.command.actor_id(),
                    "idempotency":drain.command.idempotency_id(),"create_claim":ClaimWire::new(create)?,
                    "create_actor":create.command.actor_id(),"create_idempotency":create.command.idempotency_id(),
                    "prepared":target.prepared(),"observed_progress":state}});
            if actual != &expected {
                return Err(DeploymentError::IntentMismatch);
            }
            Ok(())
        }
        _ => Err(DeploymentError::ReconciliationRequired),
    }
}
