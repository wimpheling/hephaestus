use identity_domain::AuthenticatedIdentity;
use recipe_application::{
    AttemptProvenance, DeploymentError, DeploymentLifecycle, DeploymentOperation,
    PlannedResourceIdentity, RemovalProgress, ResourceAction, ResourceOwnership, ResourceProgress,
    RetireExternalObservation,
};
use serde_json::json;

use crate::{
    PostgresDeploymentRepository, effect_context, effect_outcome,
    execution_transitions::{self, Change},
    repository_error,
};

impl PostgresDeploymentRepository {
    /// Retires an exact read-only external observation under admitted cleanup.
    ///
    /// No external lookup, permission, provider call, or worker evidence is used.
    /// Installation and verification history remain unchanged and unverified.
    ///
    /// # Errors
    /// Rejects denied cleanup, forged target/provenance, changed replay, or stale CAS.
    pub async fn retire_external_observation(
        &self,
        identity: &AuthenticatedIdentity,
        retirement: &RetireExternalObservation,
    ) -> Result<ResourceProgress, DeploymentError> {
        let claim = &retirement.target;
        if retirement.command.operation() != DeploymentOperation::Remove
            || retirement.provenance.actor_id != identity.user_id
            || retirement.provenance.attempt_id == claim.provenance.attempt_id
        {
            return Err(DeploymentError::InputConflict);
        }
        let mut context = self
            .execution_context(identity, retirement.command, claim.deployment_id, true)
            .await?;
        effect_outcome::exact(&mut context.tx, claim).await?;
        scope(&context, retirement)?;
        let fingerprint = execution_transitions::fingerprint(
            claim,
            "observation_retired",
            &json!([
                retirement.command,
                retirement.provenance,
                retirement.expected_deployment_version,
                retirement.expected_resource_version,
            ]),
        )?;
        let id = execution_transitions::transition_id(
            retirement.provenance.attempt_id.as_uuid(),
            "observation_retired",
        );
        if let Some(state) = execution_transitions::replay(&mut context.tx, id, fingerprint).await?
        {
            context.tx.commit().await.map_err(repository_error)?;
            return state.progress();
        }
        fresh_operation(&mut context, identity, retirement).await?;
        let before =
            effect_context::state(&mut context.tx, claim.deployment_id, &claim.resource).await?;
        if context.snapshot.version != retirement.expected_deployment_version
            || crate::hydration::unsigned(before.version)? != retirement.expected_resource_version
            || before.active_attempt != Some(claim.provenance.attempt_id.as_uuid())
            || crate::hydration::unsigned(before.generation)? != claim.generation
        {
            return Err(DeploymentError::StaleClaim);
        }
        if before.removal != RemovalProgress::Retained {
            return Err(DeploymentError::InvalidAction);
        }
        let mut after = before.clone();
        after.active_attempt = None;
        after.version = before
            .version
            .checked_add(1)
            .ok_or(DeploymentError::StaleClaim)?;
        execution_transitions::commit(
            &mut context,
            identity,
            Change {
                id,
                kind: "observation_retired",
                operation_id: retirement.provenance.attempt_id.as_uuid(),
                fingerprint,
                claim,
                before: &before,
                after: &after,
                proof: None,
            },
        )
        .await?;
        context.tx.commit().await.map_err(repository_error)?;
        after.progress()
    }
}

fn scope(
    context: &effect_context::Context<'_>,
    retirement: &RetireExternalObservation,
) -> Result<(), DeploymentError> {
    let claim = &retirement.target;
    let plan = context
        .snapshot
        .intent
        .resources()
        .get(&claim.resource)
        .ok_or(DeploymentError::Unavailable)?;
    if context.cleanup
        && matches!(
            context.snapshot.lifecycle,
            DeploymentLifecycle::Removing
                | DeploymentLifecycle::RecoveryRequired
                | DeploymentLifecycle::Removed
        )
        && claim.command.operation() == DeploymentOperation::Install
        && claim.action == ResourceAction::VerifyExternal
        && plan.ownership() == ResourceOwnership::External
        && matches!(
            plan.identity(),
            PlannedResourceIdentity::Volume {
                filesystem_uuid: None,
                ..
            }
        )
        && plan.identity() == claim.identity
        && plan.input_hash() == claim.input_hash
    {
        Ok(())
    } else {
        Err(DeploymentError::InvalidAction)
    }
}

async fn fresh_operation(
    context: &mut effect_context::Context<'_>,
    identity: &AuthenticatedIdentity,
    retirement: &RetireExternalObservation,
) -> Result<(), DeploymentError> {
    if retirement.provenance != AttemptProvenance::new(identity, retirement.provenance.attempt_id)?
    {
        return Err(DeploymentError::InputConflict);
    }
    let used: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM recipe_effect_attempts WHERE id=$1)
        OR EXISTS(SELECT 1 FROM recipe_effect_verifications WHERE verification_attempt_id=$1)
        OR EXISTS(SELECT 1 FROM recipe_execution_transitions WHERE operation_id=$1)",
    )
    .bind(retirement.provenance.attempt_id.as_uuid())
    .fetch_one(&mut *context.tx)
    .await
    .map_err(repository_error)?;
    if used {
        Err(DeploymentError::InputConflict)
    } else {
        Ok(())
    }
}
