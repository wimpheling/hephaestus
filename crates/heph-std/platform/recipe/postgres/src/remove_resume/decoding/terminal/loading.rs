//! Actual immutable producer rows under current transaction rights.
use super::{Loaded, Roots, wire, wire143};
use crate::{execution_preparation::active_effect::hydration as original, repository_error};
use identity_domain::{RequestId, UserId};
use recipe_application::{
    CommandIdentity, DeploymentCommandId, DeploymentError, DeploymentOperation, DeploymentSnapshot,
    PreparedInstanceSafetyTarget, PreparedInstanceTerminalPublisher,
};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

pub(super) async fn entry(
    tx: &mut Transaction<'_, Postgres>,
    entry: &wire143::Entry,
    roots: &Roots<'_>,
    snapshot: &DeploymentSnapshot,
) -> Result<Loaded, DeploymentError> {
    let plan = roots
        .intent
        .resources()
        .get(&entry.resource)
        .ok_or(DeploymentError::IntentMismatch)?;
    let row = original::row(tx, entry.claim.attempt_id).await?;
    let claim = original::restore(&row, snapshot, plan, &entry.resource)?;
    if crate::execution_rows::ClaimWire::new(&claim)? != entry.claim {
        return Err(DeploymentError::IntentMismatch);
    }
    if entry.state.active_attempt.is_none() {
        super::super::super::terminal_history::applied::qualify(tx, &claim, snapshot, &entry.state)
            .await?;
    }
    // The FIRST current Remove is not inserted yet. Select actual unique Create
    // rows using the independently loaded canonical preparation command instead
    // of invoking a face that requires an already committed current command.
    let candidates: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM recipe_effect_attempts WHERE deployment_id=$1 AND resource_name=$2 AND command_id=$3 AND action='create' ORDER BY id LIMIT 2")
        .bind(roots.intent.id().as_uuid()).bind(entry.resource.as_str())
        .bind(roots.preparation.command().id().as_uuid()).fetch_all(&mut **tx).await.map_err(repository_error)?;
    if candidates.len() != 1 {
        return Err(DeploymentError::ReconciliationRequired);
    }
    let row = original::row(tx, candidates[0]).await?;
    let create = original::restore(&row, snapshot, plan, &entry.resource)?;
    if create.command != roots.preparation.command()
        || create.action != recipe_application::ResourceAction::Create
    {
        return Err(DeploymentError::IntentMismatch);
    }
    let prepared = roots
        .preparation
        .instances()
        .iter()
        .find(|item| item.resource() == &entry.resource)
        .ok_or(DeploymentError::IntentMismatch)?;
    let target = PreparedInstanceSafetyTarget::new(
        create,
        prepared.clone(),
        roots.config.namespace().into(),
        roots.config.host().into(),
    )?;
    let publisher = if let Some(value) = &entry.publisher {
        let command = command(tx, wire::uuid(value, "command_id")?).await?;
        Some(PreparedInstanceTerminalPublisher::new(
            &claim,
            command,
            DeploymentCommandId::from_uuid(wire::uuid(value, "command_id")?)?,
            UserId::from_uuid(wire::uuid(value, "actor_id")?),
            RequestId::from_uuid(wire::uuid(value, "request_id")?),
            if value["admission_command_id"].is_null() {
                None
            } else {
                Some(DeploymentCommandId::from_uuid(wire::uuid(
                    value,
                    "admission_command_id",
                )?)?)
            },
        )?)
    } else {
        None
    };
    Ok(Loaded {
        wire: entry.clone(),
        target,
        claim,
        publisher,
    })
}
pub(super) async fn command(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
) -> Result<CommandIdentity, DeploymentError> {
    let (actor, key, operation): (Uuid, Uuid, String) = sqlx::query_as(
        "SELECT actor_id,idempotency_id,operation FROM recipe_deployment_commands WHERE id=$1",
    )
    .bind(id)
    .fetch_one(&mut **tx)
    .await
    .map_err(repository_error)?;
    let command = CommandIdentity::from_recorded(
        UserId::from_uuid(actor),
        RequestId::from_uuid(key),
        operation.parse::<DeploymentOperation>()?,
    )?;
    if command.id().as_uuid() != id {
        return Err(DeploymentError::IntentMismatch);
    }
    Ok(command)
}
