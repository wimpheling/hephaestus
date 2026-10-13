//! Exact current external claim comparison data; paired completion is qualified separately.

use authz_postgres::begin_actor_transaction;
use capability_domain::CapabilitySlotKey;
use identity_domain::AuthenticatedIdentity;
use recipe_application::{
    CommandIdentity, DeploymentError, DeploymentId, DeploymentLifecycle, DeploymentOperation,
    DeploymentSnapshot, EffectClaim, InstallProgress, PlannedResource, PlannedResourceIdentity,
    RemovalProgress, ResourceAction, ResourceOwnership,
};
use sqlx::{Postgres, Transaction};

use super::{authority, hydration, preparation_authority};
use crate::{
    PostgresExecutionPreparationRepository, effect_context, effect_state::State, persistence,
    repository_error,
};

impl PostgresExecutionPreparationRepository {
    /// Loads the exact current retained-external claim, including completed history.
    ///
    /// Actual current Install identity, whole preparation/configuration, project,
    /// Source and external authority are rechecked in one locked transaction.
    /// Returned history grants no provider authority or positive completion proof;
    /// the retained actor adapter must separately qualify its exact committed pair.
    /// Only pristine generation zero with no resource effect history returns None.
    ///
    /// # Errors
    /// Foreign/multiple/partial history, nonvolume or nonexternal resources,
    /// mismatched current generation, denied authority or preparation drift denies.
    pub async fn load_retained_external_verification_claim(
        &self,
        identity: &AuthenticatedIdentity,
        current_command: CommandIdentity,
        deployment: DeploymentId,
        resource: &CapabilitySlotKey,
    ) -> Result<Option<EffectClaim>, DeploymentError> {
        current_command.validate(identity, DeploymentOperation::Install)?;
        let mut tx = begin_actor_transaction(&self.repository.pool, identity)
            .await
            .map_err(repository_error)?;
        preparation_authority::active(&mut tx, identity).await?;
        let project =
            preparation_authority::project(&self.repository, &mut tx, identity, deployment).await?;
        persistence::lock_project(&mut tx, project).await?;
        preparation_authority::lock(&mut tx, deployment).await?;
        preparation_authority::active(&mut tx, identity).await?;
        let snapshot = crate::hydration::load(&mut tx, deployment).await?;
        authority::current(self, &mut tx, identity, current_command, &snapshot).await?;
        if !matches!(
            snapshot.lifecycle,
            DeploymentLifecycle::Installing
                | DeploymentLifecycle::Installed
                | DeploymentLifecycle::RecoveryRequired
        ) {
            return Err(DeploymentError::InvalidAction);
        }
        let plan = snapshot
            .intent
            .resources()
            .get(resource)
            .ok_or(DeploymentError::Unavailable)?;
        if plan.ownership() != ResourceOwnership::External
            || !matches!(plan.identity(), PlannedResourceIdentity::Volume { .. })
        {
            return Err(DeploymentError::InvalidAction);
        }
        let state = effect_context::state(&mut tx, deployment, resource).await?;
        let active = hydration::load(
            &mut tx,
            &snapshot,
            plan,
            resource,
            &state,
            current_command,
            None,
        )
        .await?;
        let claim = if let Some(claim) = active {
            if selected(&mut tx, &snapshot, resource, &state).await?
                != claim.provenance.attempt_id.as_uuid()
            {
                return Err(DeploymentError::ReconciliationRequired);
            }
            Some(exact(claim, current_command, &state)?)
        } else if state.generation == 0 {
            pristine(&mut tx, deployment, resource).await?;
            None
        } else {
            Some(completed(&mut tx, &snapshot, plan, resource, &state, current_command).await?)
        };
        tx.commit().await.map_err(repository_error)?;
        Ok(claim)
    }
}

async fn pristine(
    tx: &mut Transaction<'_, Postgres>,
    deployment: DeploymentId,
    resource: &CapabilitySlotKey,
) -> Result<(), DeploymentError> {
    // Do not filter by command, action or generation and turn orphan history into absence.
    let history: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM recipe_effect_attempts WHERE deployment_id=$1 AND resource_name=$2)
         OR EXISTS(SELECT 1 FROM recipe_execution_transitions WHERE deployment_id=$1 AND resource_name=$2)",
    ).bind(deployment.as_uuid()).bind(resource.as_str())
        .fetch_one(&mut **tx).await.map_err(repository_error)?;
    if history {
        return Err(DeploymentError::ReconciliationRequired);
    }
    Ok(())
}

async fn completed(
    tx: &mut Transaction<'_, Postgres>,
    snapshot: &DeploymentSnapshot,
    plan: &PlannedResource,
    resource: &CapabilitySlotKey,
    state: &State,
    command: CommandIdentity,
) -> Result<EffectClaim, DeploymentError> {
    if state.install != InstallProgress::Ready
        || state.removal != RemovalProgress::Retained
        || state.last_action != Some(ResourceAction::VerifyExternal)
        || state.diagnostic.is_some()
    {
        return Err(DeploymentError::ReconciliationRequired);
    }
    let attempt = selected(tx, snapshot, resource, state).await?;
    let row = hydration::row(tx, attempt).await?;
    let claim = hydration::restore(&row, snapshot, plan, resource)
        .map_err(|_| DeploymentError::ReconciliationRequired)?;
    exact(claim, command, state)
}

async fn selected(
    tx: &mut Transaction<'_, Postgres>,
    snapshot: &DeploymentSnapshot,
    resource: &CapabilitySlotKey,
    state: &State,
) -> Result<uuid::Uuid, DeploymentError> {
    // Foreign current/future attempts remain visible to qualification rather than
    // disappearing behind an exact-command/action filter. Older valid generations
    // are comparison history, never the current claim selected for replay.
    let attempts: Vec<uuid::Uuid> = sqlx::query_scalar(
        "SELECT id FROM recipe_effect_attempts WHERE deployment_id=$1 AND resource_name=$2
         AND generation>=$3 ORDER BY generation,id LIMIT 2",
    )
    .bind(snapshot.intent.id().as_uuid())
    .bind(resource.as_str())
    .bind(state.generation)
    .fetch_all(&mut **tx)
    .await
    .map_err(repository_error)?;
    let [attempt] = attempts.as_slice() else {
        return Err(DeploymentError::ReconciliationRequired);
    };
    Ok(*attempt)
}

fn exact(
    claim: EffectClaim,
    command: CommandIdentity,
    state: &State,
) -> Result<EffectClaim, DeploymentError> {
    if claim.command != command
        || claim.action != ResourceAction::VerifyExternal
        || claim.generation != crate::hydration::unsigned(state.generation)?
        || claim.resource_version > crate::hydration::unsigned(state.version)?
    {
        return Err(DeploymentError::ReconciliationRequired);
    }
    Ok(claim)
}
