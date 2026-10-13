//! Selected originals are read only after current protected manager qualification.
use super::super::{mapping, storage};
use super::{birth, current, ledger};
use crate::{execution_preparation, hydration, receipts, remove_resume::context, repository_error};
use capability_domain::CapabilitySlotKey;
use identity_domain::AuthenticatedIdentity;
use recipe_application::{
    CommandIdentity, DeploymentError, DeploymentIntent, DeploymentLifecycle, EffectClaim,
    PreparedOwnedVolumeCreation, RemoveDeployment, RemoveResumeConfiguration,
};
use sqlx::{Postgres, Transaction};

pub(super) async fn read(
    tx: &mut Transaction<'_, Postgres>,
    identity: &AuthenticatedIdentity,
    command: CommandIdentity,
    intent: &DeploymentIntent,
    resource: &CapabilitySlotKey,
    config: &RemoveResumeConfiguration,
) -> Result<(EffectClaim, PreparedOwnedVolumeCreation), DeploymentError> {
    super::actor(tx, identity).await?;
    let snapshot = hydration::load(tx, intent.id()).await?;
    if snapshot.intent != *intent
        || !matches!(
            snapshot.lifecycle,
            DeploymentLifecycle::Removing
                | DeploymentLifecycle::RecoveryRequired
                | DeploymentLifecycle::Removed
        )
    {
        return Err(DeploymentError::IntentMismatch);
    }
    // This existing protected actor face repeats current ALL-owned rights
    // and positively qualifies the original closure/progress comparison data.
    let observed: serde_json::Value =
        sqlx::query_scalar("SELECT public.remove_resume_context118($1)")
            .bind(intent.id().as_uuid())
            .fetch_one(&mut **tx)
            .await
            .map_err(repository_error)?;
    if !observed.is_object() {
        return Err(DeploymentError::ReconciliationRequired);
    }
    context::configured(tx, intent, config).await?;
    context::command_configuration(tx, command, config).await?;
    let receipt = receipts::find(tx, command)
        .await?
        .ok_or(DeploymentError::Unavailable)?;
    if !matches!(
        receipt.receipt_lifecycle.as_str(),
        "removing" | "recovery_required"
    ) {
        return Err(DeploymentError::ReconciliationRequired);
    }
    let input = match context::current_input(tx, command, &snapshot, Some(config)).await? {
        Some(input) => input,
        None => RemoveDeployment {
            command,
            deployment_id: intent.id(),
            expected_version: hydration::unsigned(receipt.receipt_version)?
                .checked_sub(1)
                .ok_or(DeploymentError::ReconciliationRequired)?,
        }
        .input_hash()?,
    };
    receipts::restore(&receipt, command, &snapshot, input)?;
    ledger::command_event(tx, command).await?;
    let whole = execution_preparation::hydration::load(tx, intent)
        .await?
        .ok_or(DeploymentError::ReconciliationRequired)?
        .preparation;
    whole.validate_configuration(config.profile(), config.platform())?;
    // Observe all resource attempts, rather than hiding foreign actions or
    // commands behind the desired original's selection predicates.
    let attempts: Vec<uuid::Uuid> = sqlx::query_scalar(
        "SELECT id FROM recipe_effect_attempts WHERE deployment_id=$1 AND resource_name=$2 ORDER BY id")
        .bind(intent.id().as_uuid()).bind(resource.as_str()).fetch_all(&mut **tx).await.map_err(repository_error)?;
    let [attempt] = attempts.as_slice() else {
        return Err(DeploymentError::ReconciliationRequired);
    };
    let original = execution_preparation::active_effect::hydration::row(tx, *attempt).await?;
    let claim = execution_preparation::active_effect::hydration::restore(
        &original,
        &snapshot,
        intent
            .resources()
            .get(resource)
            .ok_or(DeploymentError::IntentMismatch)?,
        resource,
    )?;
    if claim.command != whole.command()
        || claim.action != recipe_application::ResourceAction::Create
    {
        return Err(DeploymentError::ReconciliationRequired);
    }
    let prepared = storage::load(tx, &claim)
        .await?
        .ok_or(DeploymentError::ReconciliationRequired)?;
    if prepared != mapping::derive(intent, &claim)? {
        return Err(DeploymentError::InputConflict);
    }
    let ready = ledger::ready(tx, &claim, snapshot.version).await?;
    birth::qualify(tx, &claim, &prepared).await?;
    current::qualify(tx, &snapshot, &claim, &ready, config).await?;
    Ok((claim, prepared))
}
