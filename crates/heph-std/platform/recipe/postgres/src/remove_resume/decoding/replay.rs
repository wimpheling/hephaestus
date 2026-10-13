//! Logical replay preserves FIRST admission endpoints; winner is a native observation.
use super::{Decoded, Roots, publication, targets, wire};
use recipe_application::{
    CommandIdentity, DeploymentError, DeploymentSnapshot, OriginalInstanceClosure,
};
use serde_json::{Value, json};
use sqlx::{Postgres, Transaction};

// This selection grants nothing. The caller MUST qualify every selected winner
// through load_existing_instance_closure118 before returning assembled replay.
pub fn compare(
    saved: &Decoded,
    current: &Decoded,
    snapshot: &DeploymentSnapshot,
    expected: u64,
) -> Result<(), DeploymentError> {
    super::terminal::compare_replay(&saved.terminal, &current.terminal)?;
    for original in &saved.closures {
        if !current.closures.contains(original) {
            return Err(DeploymentError::ReconciliationRequired);
        }
    }
    for entry in &current.continuations {
        if !saved
            .continuations
            .iter()
            .any(|old| old.claim == entry.claim)
        {
            return Err(DeploymentError::ReconciliationRequired);
        }
    }
    for item in &current.pending {
        if snapshot.version != expected
            || !saved.pending.iter().any(|old| old.source == item.source)
        {
            // Advancing unpublished progress requires a genuinely new admission.
            return Err(DeploymentError::ReconciliationRequired);
        }
    }
    for winner in &current.closures {
        if saved.closures.contains(winner) {
            continue;
        }
        let original = saved
            .pending
            .iter()
            .find(|old| old.drain == winner.drain)
            .ok_or(DeploymentError::ReconciliationRequired)?;
        if let recipe_application::InstanceClosurePublicationOrigin::QualifiedFreshRemove(
            publication,
        ) = &winner.publication_origin
        {
            if publication.drain() != &original.drain {
                return Err(DeploymentError::IntentMismatch);
            }
        }
    }
    for original in &saved.pending {
        if !current
            .pending
            .iter()
            .any(|item| item.source == original.source)
            && !current
                .closures
                .iter()
                .any(|closure| closure.drain == original.drain)
        {
            return Err(DeploymentError::ReconciliationRequired);
        }
    }
    Ok(())
}

pub async fn observe(
    tx: &mut Transaction<'_, Postgres>,
    command: CommandIdentity,
    roots: &Roots<'_>,
    saved: &Decoded,
    current: &Decoded,
    data: &Value,
) -> Result<(), DeploymentError> {
    // Both original closed entries and newly published Pending winners requalify
    // the immutable admission prefix via the SAME actual current command/pool.
    for closure in &current.closures {
        let target = if let Some(original) = saved
            .pending
            .iter()
            .find(|item| item.drain == closure.drain)
        {
            original.target.clone()
        } else {
            targets::load(tx, command, roots, &closure.drain.resource).await?
        };
        let create = target.original();
        let actual: Value = sqlx::query_scalar(
            "SELECT load_existing_instance_closure118($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)",
        )
        .bind(command.id().as_uuid())
        .bind(roots.intent.id().as_uuid())
        .bind(roots.intent.project_id().as_uuid())
        .bind(roots.intent.input_hash().as_bytes().as_slice())
        .bind(create.provenance.attempt_id.as_uuid())
        .bind(
            serde_json::to_value(crate::execution_rows::ClaimWire::new(create)?)
                .map_err(|_| DeploymentError::Serialization)?,
        )
        .bind(serde_json::to_value(target.prepared()).map_err(|_| DeploymentError::Serialization)?)
        .bind(roots.config.namespace())
        .bind(roots.config.host())
        .bind(
            serde_json::to_value(roots.config.platform())
                .map_err(|_| DeploymentError::Serialization)?,
        )
        .fetch_one(&mut **tx)
        .await
        .map_err(super::storage)?;
        let source = wire::array(data, "closures", roots.intent.resources().len())?
            .iter()
            .find(|value| value["resource"].as_str() == Some(closure.drain.resource.as_str()))
            .ok_or(DeploymentError::ReconciliationRequired)?;
        if actual != expected(closure, roots, source)? {
            return Err(DeploymentError::ReconciliationRequired);
        }
    }
    Ok(())
}

fn expected(
    closure: &OriginalInstanceClosure,
    roots: &Roots<'_>,
    source: &Value,
) -> Result<Value, DeploymentError> {
    let mut value = json!({"drain":crate::execution_rows::ClaimWire::new(&closure.drain)?,
        "actor":closure.drain.command.actor_id(),"idempotency":closure.drain.command.idempotency_id(),
        "removal_id":closure.removal_id,"command_key":closure.command_key,"expected_version":closure.expected_version,
        "creator":closure.actor,"request":closure.request,"input_hash":closure.input_hash});
    if let Some(publication) = publication::encode(&closure.publication_origin, roots, source)? {
        value["publication"] = publication;
    }
    Ok(value)
}

pub fn receipt(lifecycle: &str, saved: &Decoded) -> Result<(), DeploymentError> {
    let recovery = saved.terminal.recovery()
        || !saved.recovery.is_empty()
        || saved.pending.iter().any(|item| {
            item.state.removal == recipe_application::RemovalProgress::RecoveryRequired
        });
    match lifecycle {
        "removing" if !recovery => Ok(()),
        "recovery_required" if recovery => Ok(()),
        _ => Err(DeploymentError::ReconciliationRequired),
    }
}
