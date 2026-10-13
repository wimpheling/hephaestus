//! Full native row bytes come from this transaction, never from invented Core fields.
use super::transaction;
use crate::{decode_hash, prepared_instance_safety};
use recipe_application::{DeploymentError, OriginalInstanceClosure, PreparedInstanceSafetyTarget};
use serde_json::{Value, json};
use sqlx::{Postgres, Transaction};

pub async fn actor(
    tx: &mut Transaction<'_, Postgres>,
    target: &PreparedInstanceSafetyTarget,
    expected: &OriginalInstanceClosure,
) -> Result<Value, DeploymentError> {
    // This protected app frontend independently repeats current ALL-owned rights
    // and qualifies the whole native original inventory before returning context.
    let context: Value = sqlx::query_scalar("SELECT remove_resume_context118($1)")
        .bind(target.original().deployment_id.as_uuid())
        .fetch_one(&mut **tx)
        .await
        .map_err(|error| transaction::storage(&error))?;
    let closures = context["closures"]
        .as_array()
        .ok_or(DeploymentError::ReconciliationRequired)?;
    let matching: Vec<_> = closures
        .iter()
        .filter(|value| value["resource"].as_str() == Some(expected.drain.resource.as_str()))
        .collect();
    let [value] = matching.as_slice() else {
        return Err(DeploymentError::ReconciliationRequired);
    };
    checked((*value).clone(), target, expected)
}

pub async fn worker(
    tx: &mut Transaction<'_, Postgres>,
    target: &PreparedInstanceSafetyTarget,
    expected: &OriginalInstanceClosure,
) -> Result<Value, DeploymentError> {
    // All values, including timestamps/bytea and the saved Pending endpoint, are
    // actual granted immutable rows in this SAME RRRO snapshot. This constructs
    // comparison data; the worker143 frontend then requalifies the full object.
    let rows: Vec<Value> = sqlx::query_scalar(
        "SELECT jsonb_build_object('resource',drain.resource_name,'drain_claim',drain.claim_json,
          'actor',original.actor_id,'idempotency',original.idempotency_id,
          'removal',to_jsonb(removal)-'prepared_resume_command_id','inbox',to_jsonb(inbox)) ||
          CASE WHEN removal.prepared_resume_command_id IS NULL THEN '{}'::jsonb ELSE
          jsonb_build_object('publication',jsonb_build_object('kind','qualified_fresh_remove',
            'command_id',publisher.id,'actor',publisher.actor_id,'idempotency',publisher.idempotency_id,
            'admission_request',admission.request_id,'publication_request',removal.request_id,
            'admission_version',admission.expected_version,'pending',
            (SELECT entry FROM jsonb_array_elements(admission.context_json->'pending') entry
              WHERE entry->>'resource'=drain.resource_name))) END
         FROM agent_instance_removal_requests removal
         JOIN release_command_inbox inbox ON inbox.secondary_id=removal.id
         JOIN recipe_effect_attempts drain ON drain.id=$2 AND drain.deployment_id=$3
         JOIN recipe_deployment_commands original ON original.id=drain.command_id
         LEFT JOIN recipe_remove_resume_admissions admission ON admission.command_id=removal.prepared_resume_command_id
         LEFT JOIN recipe_deployment_commands publisher ON publisher.id=admission.command_id
         WHERE removal.instance_id=$1",
    ).bind(target.prepared().instance_id().as_uuid())
        .bind(expected.drain.provenance.attempt_id.as_uuid())
        .bind(target.original().deployment_id.as_uuid())
        .fetch_all(&mut **tx).await.map_err(|error| transaction::storage(&error))?;
    let [value] = rows.as_slice() else {
        return Err(DeploymentError::ReconciliationRequired);
    };
    checked(value.clone(), target, expected)
}

pub(super) fn checked(
    value: Value,
    target: &PreparedInstanceSafetyTarget,
    expected: &OriginalInstanceClosure,
) -> Result<Value, DeploymentError> {
    expected.validate(target.original())?;
    let mut keys = vec![
        "resource",
        "drain_claim",
        "actor",
        "idempotency",
        "removal",
        "inbox",
    ];
    if value.get("publication").is_some() {
        keys.push("publication");
    }
    fields(&value, &keys)?;
    let drain = &expected.drain;
    if value["resource"] != json!(drain.resource)
        || value["drain_claim"] != prepared_instance_safety::claim(drain)?
        || value["actor"] != json!(drain.command.actor_id())
        || value["idempotency"] != json!(drain.command.idempotency_id())
        || prepared_instance_safety::publication(value.get("publication"), target, drain)?
            != expected.publication_origin
    {
        return Err(DeploymentError::IntentMismatch);
    }
    let removal = &value["removal"];
    fields(
        removal,
        &[
            "id",
            "instance_id",
            "expected_version",
            "input_hash",
            "created_by",
            "request_id",
            "authorization_model_version",
            "created_at",
        ],
    )?;
    let inbox = &value["inbox"];
    fields(
        inbox,
        &[
            "command_key",
            "operation",
            "aggregate_id",
            "secondary_id",
            "actor_id",
            "request_id",
            "completed_at",
            "input_hash",
        ],
    )?;
    if removal["id"] != json!(expected.removal_id)
        || removal["instance_id"] != json!(target.prepared().instance_id())
        || removal["expected_version"] != json!(expected.expected_version)
        || removal["created_by"] != json!(expected.actor)
        || removal["request_id"] != json!(expected.request)
        || bytea(removal, "input_hash")? != *expected.input_hash.as_bytes()
        || inbox["command_key"].is_null()
        || bytea(inbox, "command_key")? != *expected.command_key.as_bytes()
        || inbox["operation"] != "request_instance_removal"
        || inbox["aggregate_id"] != removal["instance_id"]
        || inbox["secondary_id"] != removal["id"]
        || inbox["actor_id"] != removal["created_by"]
        || inbox["request_id"] != removal["request_id"]
        || bytea(inbox, "input_hash")? != *expected.input_hash.as_bytes()
    {
        return Err(DeploymentError::IntentMismatch);
    }
    // Full original JSON remains unchanged; unrepresented native row fields are
    // still carried into, and independently qualified by, the protected frontend.
    Ok(value)
}
pub(super) fn fields(value: &Value, names: &[&str]) -> Result<(), DeploymentError> {
    if value.as_object().is_none_or(|object| {
        object.len() != names.len() || names.iter().any(|name| !object.contains_key(*name))
    }) {
        return Err(DeploymentError::ReconciliationRequired);
    }
    Ok(())
}
fn bytea(value: &Value, name: &str) -> Result<[u8; 32], DeploymentError> {
    let hex = value[name]
        .as_str()
        .and_then(|value| value.strip_prefix("\\x"))
        .ok_or(DeploymentError::ReconciliationRequired)?;
    decode_hash(hex).map_err(|_| DeploymentError::ReconciliationRequired)
}
