use capability_domain::CapabilitySlotKey;
use identity_domain::{RequestId, UserId};
use recipe_application::{
    AttemptProvenance, CommandIdentity, DeploymentAttemptId, DeploymentError, DeploymentOperation,
    DeploymentSnapshot, EffectClaim, InstallProgress, PlannedResource, RemovalProgress,
};
use release_domain::ContentHash;
use sqlx::{Postgres, Transaction};

use super::rows::ActiveRow;
use crate::{
    effect_state::{self, State},
    execution_rows::ClaimWire,
    repository_error,
};

pub async fn load(
    tx: &mut Transaction<'_, Postgres>,
    snapshot: &DeploymentSnapshot,
    plan: &PlannedResource,
    name: &CapabilitySlotKey,
    state: &State,
    current: CommandIdentity,
    config: Option<&recipe_application::RemoveResumeConfiguration>,
) -> Result<Option<EffectClaim>, DeploymentError> {
    let Some(attempt) = state.active_attempt else {
        inactive(tx, snapshot, plan, name, state).await?;
        return Ok(None);
    };
    let row = row(tx, attempt).await?;
    let claim =
        restore(&row, snapshot, plan, name).map_err(|_| DeploymentError::ReconciliationRequired)?;
    let qualified = current != claim.command
        && crate::remove_resume::selected(tx, current, &claim, config).await?;
    if claim.generation
        != crate::hydration::unsigned(state.generation)
            .map_err(|_| DeploymentError::ReconciliationRequired)?
        || claim.resource_version
            > crate::hydration::unsigned(state.version)
                .map_err(|_| DeploymentError::ReconciliationRequired)?
        || ((current.operation() == DeploymentOperation::Install
            || claim.command.operation() == DeploymentOperation::Remove)
            && claim.command != current
            && !qualified)
    {
        return Err(DeploymentError::ReconciliationRequired);
    }
    if claim.command.operation() == DeploymentOperation::Install
        && crate::execution_preparation::authority::original(tx, &snapshot.intent)
            .await
            .map_err(|_| DeploymentError::ReconciliationRequired)?
            != claim.command
    {
        return Err(DeploymentError::ReconciliationRequired);
    }
    current_state(tx, snapshot, name, state).await?;
    Ok(Some(claim))
}

pub async fn row(
    tx: &mut Transaction<'_, Postgres>,
    attempt: uuid::Uuid,
) -> Result<ActiveRow, DeploymentError> {
    sqlx::query_as::<_, ActiveRow>(
        "SELECT attempt.id,attempt.command_id,attempt.deployment_id,attempt.project_id,
          attempt.resource_name,attempt.action,attempt.generation,attempt.input_hash,attempt.claim_json,
          attempt.before_state,attempt.request_fingerprint,attempt.expected_deployment_version,
          attempt.actor_id,attempt.request_id,command.actor_id AS command_actor,command.idempotency_id,
          command.operation,transition.after_state AS claimed_state,
          (transition.id IS NOT NULL AND transition.command_id=attempt.command_id
           AND transition.operation_id=attempt.id AND transition.deployment_id=attempt.deployment_id
           AND transition.project_id=attempt.project_id AND transition.resource_name=attempt.resource_name
           AND transition.actor_id=attempt.actor_id AND transition.request_id=attempt.request_id
           AND transition.before_state=attempt.before_state AND transition.fingerprint=attempt.request_fingerprint
           AND transition.before_deployment_version=attempt.expected_deployment_version
           AND EXISTS(SELECT 1 FROM application_events event JOIN product_event_outbox outbox
             ON outbox.event_id=event.id WHERE event.id=transition.event_id
             AND event.scope_kind='project' AND event.scope_id=attempt.project_id
             AND event.aggregate_type='project' AND event.aggregate_id=attempt.project_id
             AND event.event_type='project.changed' AND event.change_kind='updated'
             AND event.actor_type='user' AND event.actor_id=attempt.actor_id AND event.request_id=attempt.request_id
             AND event.cursor=transition.event_cursor AND event.aggregate_version=transition.event_aggregate_version)) AS proof_valid
         FROM recipe_effect_attempts attempt JOIN recipe_deployment_commands command
           ON command.id=attempt.command_id AND command.deployment_id=attempt.deployment_id
           AND command.project_id=attempt.project_id
         LEFT JOIN recipe_execution_transitions transition ON transition.attempt_id=attempt.id AND transition.kind='claimed'
         WHERE attempt.id=$1",
    ).bind(attempt).fetch_optional(&mut **tx).await.map_err(repository_error)?
        .ok_or(DeploymentError::ReconciliationRequired)
}

pub fn restore(
    row: &ActiveRow,
    snapshot: &DeploymentSnapshot,
    plan: &PlannedResource,
    resource: &CapabilitySlotKey,
) -> Result<EffectClaim, DeploymentError> {
    let wire: ClaimWire = serde_json::from_value(row.claim_json.0.clone())
        .map_err(|_| DeploymentError::ReconciliationRequired)?;
    let command = CommandIdentity::from_recorded(
        UserId::from_uuid(row.command_actor),
        RequestId::from_uuid(row.idempotency_id),
        row.operation.parse()?,
    )?;
    if serde_json::to_value(&wire).map_err(|_| DeploymentError::Serialization)? != row.claim_json.0
    {
        return Err(DeploymentError::ReconciliationRequired);
    }
    let claim = wire.restore(command, snapshot, plan, resource)?;
    let before: State = serde_json::from_value(row.before_state.0.clone())
        .map_err(|_| DeploymentError::ReconciliationRequired)?;
    effect_state::legality(plan, &before, command.operation(), claim.action)
        .map_err(|_| DeploymentError::ReconciliationRequired)?;
    let request = serde_json::to_vec(&(
        command,
        claim.deployment_id,
        &claim.resource,
        row.expected_deployment_version,
        before.version,
        claim.action,
        claim.provenance,
    ))
    .map_err(|_| DeploymentError::Serialization)?;
    let after = before.claim(&claim)?;
    if row.id != wire.attempt_id
        || row.command_id != command.id().as_uuid()
        || row.deployment_id != snapshot.intent.id().as_uuid()
        || row.project_id != snapshot.intent.project_id().as_uuid()
        || row.resource_name != claim.resource.as_str()
        || row.action != claim.action.as_str()
        || row.generation <= 0
        || u64::try_from(row.generation).ok() != Some(claim.generation)
        || row.input_hash.as_slice() != claim.input_hash.as_bytes()
        || row.actor_id != claim.provenance.actor_id.as_uuid()
        || row.request_id != claim.provenance.request_id.as_uuid()
        || row.expected_deployment_version < 0
        || row.expected_deployment_version
            >= i64::try_from(snapshot.version)
                .map_err(|_| DeploymentError::ReconciliationRequired)?
        || before
            .version
            .checked_add(1)
            .and_then(|value| u64::try_from(value).ok())
            != Some(claim.resource_version)
        || before
            .generation
            .checked_add(1)
            .and_then(|value| u64::try_from(value).ok())
            != Some(claim.generation)
        || row.request_fingerprint.as_slice() != ContentHash::digest(&request).as_bytes()
        || row.claimed_state.as_ref().map(|state| &state.0)
            != Some(&serde_json::to_value(after).map_err(|_| DeploymentError::Serialization)?)
        || !row.proof_valid
    {
        return Err(DeploymentError::ReconciliationRequired);
    }
    Ok(claim)
}

impl ClaimWire {
    fn restore(
        &self,
        command: CommandIdentity,
        snapshot: &DeploymentSnapshot,
        plan: &PlannedResource,
        resource: &CapabilitySlotKey,
    ) -> Result<EffectClaim, DeploymentError> {
        if self.actor_id.is_nil() || self.request_id.is_nil() {
            return Err(DeploymentError::ReconciliationRequired);
        }
        let claim = EffectClaim {
            command,
            deployment_id: snapshot.intent.id(),
            resource: resource.clone(),
            identity: plan.identity(),
            input_hash: plan.input_hash(),
            resource_version: self.resource_version,
            generation: self.generation,
            action: self.action,
            provenance: AttemptProvenance {
                attempt_id: DeploymentAttemptId::from_uuid(self.attempt_id)?,
                actor_id: UserId::from_uuid(self.actor_id),
                request_id: RequestId::from_uuid(self.request_id),
            },
        };
        if &Self::new(&claim)? != self
            || claim.provenance.actor_id != command.actor_id()
            || claim.resource_version == 0
            || claim.generation == 0
        {
            return Err(DeploymentError::ReconciliationRequired);
        }
        plan.validate_action(claim.action)
            .map_err(|_| DeploymentError::ReconciliationRequired)?;
        Ok(claim)
    }
}

async fn inactive(
    tx: &mut Transaction<'_, Postgres>,
    snapshot: &DeploymentSnapshot,
    plan: &PlannedResource,
    resource: &CapabilitySlotKey,
    state: &State,
) -> Result<(), DeploymentError> {
    if matches!(
        state.install,
        InstallProgress::Creating | InstallProgress::RecoveryRequired
    ) || state.removal == RemovalProgress::RecoveryRequired
    {
        return Err(DeploymentError::ReconciliationRequired);
    }
    if state.generation == 0 {
        let initial_removal = match plan.ownership() {
            recipe_application::ResourceOwnership::Owned => RemovalProgress::Pending,
            recipe_application::ResourceOwnership::External => RemovalProgress::Retained,
        };
        if state.version != 0
            || state.install != InstallProgress::Pending
            || state.removal != initial_removal
            || state.diagnostic.is_some()
            || state.last_action.is_some()
        {
            return Err(DeploymentError::ReconciliationRequired);
        }
        return Ok(());
    }
    current_state(tx, snapshot, resource, state).await
}

pub async fn current_state(
    tx: &mut Transaction<'_, Postgres>,
    snapshot: &DeploymentSnapshot,
    resource: &CapabilitySlotKey,
    state: &State,
) -> Result<(), DeploymentError> {
    let valid: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM recipe_execution_transitions transition JOIN recipe_effect_attempts attempt
          ON attempt.id=transition.attempt_id AND attempt.deployment_id=transition.deployment_id
          AND attempt.project_id=transition.project_id AND attempt.resource_name=transition.resource_name
         WHERE transition.deployment_id=$1 AND transition.project_id=$2 AND transition.resource_name=$3
           AND transition.after_state=$4 AND transition.after_deployment_version<=$5 AND attempt.generation=$6)",
    ).bind(snapshot.intent.id().as_uuid()).bind(snapshot.intent.project_id().as_uuid())
        .bind(resource.as_str()).bind(sqlx::types::Json(state))
        .bind(i64::try_from(snapshot.version).map_err(|_| DeploymentError::ReconciliationRequired)?)
        .bind(state.generation).fetch_one(&mut **tx).await.map_err(repository_error)?;
    if !valid {
        return Err(DeploymentError::ReconciliationRequired);
    }
    Ok(())
}

#[cfg(test)]
#[path = "decoding_tests.rs"]
mod tests;
