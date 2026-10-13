mod transaction;
pub use transaction::{complete, reconcile};

use crate::{
    PostgresDeploymentRepository, effect_context,
    effect_state::State,
    execution_rows::{AttemptRow, ClaimWire},
    execution_transitions::{self, Change},
    repository_error,
};
use identity_domain::AuthenticatedIdentity;
use recipe_application::{
    AttemptProvenance, CommandIdentity, DeploymentError, DeploymentLifecycle, DeploymentOperation,
    DiagnosticCode, EffectClaim, EffectEvidence, ReconciledOutcome, ResourceOwnership,
    ResourceProgress,
};
use serde_json::json;

impl PostgresDeploymentRepository {
    /// Records a definite result only after matching trusted worker verification.
    ///
    /// # Errors
    /// Rejects denied access, forged evidence, stale claims, or absent verified proof.
    pub async fn complete_resource_effect(
        &self,
        identity: &AuthenticatedIdentity,
        claim: &EffectClaim,
        evidence: EffectEvidence,
    ) -> Result<ResourceProgress, DeploymentError> {
        let mut context = self
            .execution_context(identity, claim.command, claim.deployment_id, true)
            .await?;
        let after = complete(&mut context, identity, claim, evidence).await?;
        context.tx.commit().await.map_err(repository_error)?;
        after.progress()
    }

    /// Preserves ambiguity and the original active fence until reconciliation.
    ///
    /// # Errors
    /// Rejects denied access, forged/stale claims, changed retries, or persistence failure.
    pub async fn mark_resource_ambiguous(
        &self,
        identity: &AuthenticatedIdentity,
        claim: &EffectClaim,
        diagnostic: DiagnosticCode,
    ) -> Result<ResourceProgress, DeploymentError> {
        let mut context = self
            .execution_context(identity, claim.command, claim.deployment_id, true)
            .await?;
        exact(&mut context.tx, claim).await?;
        let fingerprint =
            execution_transitions::fingerprint(claim, "ambiguous", &json!(diagnostic))?;
        let id = execution_transitions::transition_id(
            claim.provenance.attempt_id.as_uuid(),
            "ambiguous",
        );
        if let Some(state) = execution_transitions::replay(&mut context.tx, id, fingerprint).await?
        {
            context.tx.commit().await.map_err(repository_error)?;
            return state.progress();
        }
        let before = active(&mut context.tx, claim).await?;
        if crate::hydration::unsigned(before.version)? != claim.resource_version {
            return Err(DeploymentError::StaleClaim);
        }
        let mut after = before.uncertain(claim.action, diagnostic);
        after.version = increment(before.version)?;
        execution_transitions::commit(
            &mut context,
            identity,
            Change {
                id,
                kind: "ambiguous",
                operation_id: claim.provenance.attempt_id.as_uuid(),
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

    /// Records a fresh observation against the original immutable fenced claim.
    ///
    /// # Errors
    /// Rejects denied access, stale/forged provenance, contradictory retries, or missing proof.
    pub async fn record_reconciliation(
        &self,
        identity: &AuthenticatedIdentity,
        command: CommandIdentity,
        claim: &EffectClaim,
        provenance: AttemptProvenance,
        outcome: ReconciledOutcome,
    ) -> Result<ResourceProgress, DeploymentError> {
        if provenance != AttemptProvenance::new(identity, provenance.attempt_id)?
            || provenance.attempt_id == claim.provenance.attempt_id
        {
            return Err(DeploymentError::InputConflict);
        }
        let mut context = self
            .execution_context(identity, command, claim.deployment_id, true)
            .await?;
        let after = reconcile(
            self,
            &mut context,
            identity,
            command,
            claim,
            provenance,
            outcome,
        )
        .await?;
        context.tx.commit().await.map_err(repository_error)?;
        after.progress()
    }
}

pub async fn exact(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    claim: &EffectClaim,
) -> Result<AttemptRow, DeploymentError> {
    let row: AttemptRow =
        sqlx::query_as("SELECT claim_json, before_state FROM recipe_effect_attempts WHERE id=$1")
            .bind(claim.provenance.attempt_id.as_uuid())
            .fetch_optional(&mut **tx)
            .await
            .map_err(repository_error)?
            .ok_or(DeploymentError::Unavailable)?;
    if row.claim_json.0 != ClaimWire::new(claim)? {
        return Err(DeploymentError::InputConflict);
    }
    Ok(row)
}

async fn active(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    claim: &EffectClaim,
) -> Result<State, DeploymentError> {
    let state = effect_context::state(tx, claim.deployment_id, &claim.resource).await?;
    if state.active_attempt != Some(claim.provenance.attempt_id.as_uuid())
        || crate::hydration::unsigned(state.generation)? != claim.generation
    {
        return Err(DeploymentError::StaleClaim);
    }
    Ok(state)
}

fn increment(version: i64) -> Result<i64, DeploymentError> {
    version.checked_add(1).ok_or(DeploymentError::StaleClaim)
}

fn reconciliation_scope(
    context: &effect_context::Context<'_>,
    command: CommandIdentity,
    claim: &EffectClaim,
) -> Result<(), DeploymentError> {
    if command == claim.command {
        return Ok(());
    }
    let owned = context
        .snapshot
        .intent
        .resources()
        .get(&claim.resource)
        .is_some_and(|resource| resource.ownership() == ResourceOwnership::Owned);
    if command.operation() == DeploymentOperation::Remove
        && claim.command.operation() == DeploymentOperation::Install
        && context.cleanup
        && owned
        && matches!(
            context.snapshot.lifecycle,
            DeploymentLifecycle::Removing
                | DeploymentLifecycle::RecoveryRequired
                | DeploymentLifecycle::Removed
        )
    {
        Ok(())
    } else {
        Err(DeploymentError::InvalidAction)
    }
}
