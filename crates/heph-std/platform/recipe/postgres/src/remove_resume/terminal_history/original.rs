//! Cold original-command replay requires positive inactive Applied history.
#[cfg(test)]
#[path = "original/tests.rs"]
mod tests;

use super::super::decoding;
use super::{Reader, applied, closure, encoding, role, storage, worker};
use capability_domain::CapabilitySlotKey;
use identity_domain::AuthenticatedIdentity;
use recipe_application::{
    CommandIdentity, DeploymentError, DeploymentIntent, DeploymentOperation,
    PreparedInstanceDeleteHistory, PreparedInstanceDeleteHistoryState,
};
use serde_json::{Value, json};

pub(super) async fn load(
    reader: &Reader,
    identity: &AuthenticatedIdentity,
    intent: &DeploymentIntent,
    current: CommandIdentity,
    resource: &CapabilitySlotKey,
) -> Result<PreparedInstanceDeleteHistory, DeploymentError> {
    current.validate(identity, DeploymentOperation::Remove)?;
    if !intent.resources().contains_key(resource) {
        return Err(DeploymentError::IntentMismatch);
    }
    let mut tx = reader
        .actor
        .begin_with("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .await
        .map_err(storage)?;
    let result = async {
        role(&mut tx, "hephaestus_app").await?;
        sqlx::query("SELECT set_config('hephaestus.actor_id',$1,true),set_config('hephaestus.subject_type','user',true),set_config('hephaestus.request_id',$2,true),set_config('hephaestus.occurrence_id',$3,true)")
            .bind(identity.user_id.to_string()).bind(identity.request_id.to_string())
            .bind(identity.idempotency_id.to_string()).execute(&mut *tx).await.map_err(storage)?;
        let snapshot = crate::hydration::load(&mut tx, intent.id()).await?;
        if snapshot.intent != *intent { return Err(DeploymentError::IntentMismatch); }
        let stored = crate::execution_preparation::hydration::load(&mut tx, intent).await?
            .ok_or(DeploymentError::ReconciliationRequired)?;
        let roots = decoding::Roots { intent, preparation: &stored.preparation, config: &reader.remove };
        let data: Value = sqlx::query_scalar("SELECT remove_resume_context118($1)")
            .bind(intent.id().as_uuid()).fetch_one(&mut *tx).await.map_err(storage)?;
        // Existing full decoder loads EVERY original/association and qualifies
        // every inactive Applied transition, worker proof, product event and
        // fingerprint before any selected history can be returned.
        let decoded = decoding::load(&mut tx, &data, &roots).await?;
        let complete = decoding::current(decoded, &snapshot, current)?;
        let matching: Vec<_> = complete.terminal.iter()
            .filter(|history| &history.original().original().effect().resource == resource).collect();
        let [history] = matching.as_slice() else { return Err(DeploymentError::ReconciliationRequired); };
        let request = history.original().original();
        if history.state() != PreparedInstanceDeleteHistoryState::Applied
            || request.effect().command != current || request.current_command() != current
            || request.intent() != intent || request.configuration() != &reader.remove {
            return Err(DeploymentError::ReconciliationRequired);
        }
        let raw = selected(&data, current, resource)?;
        let closures = data["closures"].as_array().ok_or(DeploymentError::ReconciliationRequired)?;
        let matching: Vec<_> = closures.iter().filter(|item| item["resource"] == resource.as_str()).collect();
        let [original] = matching.as_slice() else { return Err(DeploymentError::ReconciliationRequired); };
        closure::compare(&mut tx, original, request).await?;
        let expected = encoding::request(reader, request, original)?;
        // This already granted face repeats current ALL rights/config/whole
        // native original qualification, including saved before-CAS. No fresh
        // admission or identity is invented for the original command.
        let actual: Value = sqlx::query_scalar("SELECT load_prepared_instance_delete_continuation143($1,$2,$3)")
            .bind(current.id().as_uuid()).bind(&expected).bind(encoding::configuration(reader)?)
            .fetch_one(&mut *tx).await.map_err(storage)?;
        if actual.as_object().is_none_or(|object| object.len() != 3)
            || actual["version"] != 1 || actual["request"] != expected || actual["history"] != *raw {
            return Err(DeploymentError::ReconciliationRequired);
        }
        let observed = worker::decode_history(&mut tx, request, &actual["history"]).await?;
        if observed != **history || observed.state() != PreparedInstanceDeleteHistoryState::Applied
            || observed.observed_progress() != &applied::observe(&mut tx, request).await? {
            return Err(DeploymentError::ReconciliationRequired);
        }
        Ok(observed)
    }.await;
    let rollback = tx.rollback().await.map_err(storage);
    match result {
        Err(primary) => Err(primary),
        Ok(value) => rollback.map(|()| value),
    }
}

// A wire-shape check after complete hydration; it supplies no native authority.
fn selected<'a>(
    data: &'a Value,
    current: CommandIdentity,
    resource: &CapabilitySlotKey,
) -> Result<&'a Value, DeploymentError> {
    let entries = data["terminal"]
        .as_array()
        .ok_or(DeploymentError::ReconciliationRequired)?;
    let matching: Vec<_> = entries
        .iter()
        .filter(|item| item["resource"] == resource.as_str())
        .collect();
    let [raw] = matching.as_slice() else {
        return Err(DeploymentError::ReconciliationRequired);
    };
    let decoded = decoding::terminal_history_entries(&json!({"terminal":[raw]}))?;
    let [entry] = decoded.as_slice() else {
        return Err(DeploymentError::ReconciliationRequired);
    };
    if entry.claim.command_id != current.id().as_uuid()
        || entry.resource != *resource
        || entry.state.active_attempt.is_some()
        || entry.operation.is_none()
    {
        return Err(DeploymentError::ReconciliationRequired);
    }
    Ok(raw)
}
