use crate::{
    PostgresDeploymentRepository, effect_context,
    effect_state::{self, State},
    execution_rows::{AttemptRow, ClaimWire},
    execution_transitions::{self, Change},
    repository_error, verification,
};
use identity_domain::AuthenticatedIdentity;
use recipe_application::{
    AttemptProvenance, CommandIdentity, DeploymentError, DeploymentLifecycle, DeploymentOperation,
    DiagnosticCode, EffectClaim, EffectEvidence, EffectOutcome, InstallProgress, ReconciledOutcome,
    ResourceAction, ResourceOwnership, ResourceProgress,
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
        exact(&mut context.tx, claim).await?;
        if evidence.identity != claim.identity || evidence.input_hash != claim.input_hash {
            return Err(DeploymentError::InputConflict);
        }
        let (kind, diagnostic) = effect_state::outcome(evidence);
        let fingerprint =
            execution_transitions::fingerprint(claim, "completed", &json!([kind, diagnostic]))?;
        let id = execution_transitions::transition_id(
            claim.provenance.attempt_id.as_uuid(),
            "completed",
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
        let proof = verification::matching(
            &mut context.tx,
            claim,
            claim.provenance.attempt_id,
            kind,
            diagnostic,
        )
        .await?;
        let mut after = match evidence.outcome {
            EffectOutcome::Applied => before.applied(claim.action),
            EffectOutcome::Failed(code) => {
                let mut failed = before.uncertain(claim.action, code);
                if matches!(
                    claim.action,
                    ResourceAction::Create | ResourceAction::VerifyExternal
                ) {
                    failed.install = InstallProgress::Failed;
                }
                failed
            }
        };
        after.version = increment(before.version)?;
        execution_transitions::commit(
            &mut context,
            identity,
            Change {
                id,
                kind: "completed",
                operation_id: claim.provenance.attempt_id.as_uuid(),
                fingerprint,
                claim,
                before: &before,
                after: &after,
                proof: Some(proof),
            },
        )
        .await?;
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
        let original = exact(&mut context.tx, claim).await?;
        reconciliation_scope(&context, command, claim)?;
        let (kind, diagnostic) = match outcome {
            ReconciledOutcome::Applied(evidence) => {
                if evidence.identity != claim.identity
                    || evidence.input_hash != claim.input_hash
                    || evidence.outcome != EffectOutcome::Applied
                {
                    return Err(DeploymentError::InputConflict);
                }
                ("applied", None)
            }
            ReconciledOutcome::NotApplied => ("not_applied", None),
            ReconciledOutcome::Unproven(code) => ("unproven", Some(code)),
        };
        let fingerprint = execution_transitions::fingerprint(
            claim,
            "reconciled",
            &json!([command, provenance, kind, diagnostic]),
        )?;
        let id =
            execution_transitions::transition_id(provenance.attempt_id.as_uuid(), "reconciled");
        if let Some(state) = execution_transitions::replay(&mut context.tx, id, fingerprint).await?
        {
            context.tx.commit().await.map_err(repository_error)?;
            return state.progress();
        }
        let before = active(&mut context.tx, claim).await?;
        let proof = if kind == "unproven" {
            None
        } else {
            Some(
                verification::matching(
                    &mut context.tx,
                    claim,
                    provenance.attempt_id,
                    kind,
                    diagnostic,
                )
                .await?,
            )
        };
        let mut after = match outcome {
            ReconciledOutcome::Applied(_) => before.applied(claim.action),
            ReconciledOutcome::Unproven(code) => before.uncertain(claim.action, code),
            ReconciledOutcome::NotApplied => {
                let mut restored = original.before_state.0;
                restored.generation = before.generation;
                restored.active_attempt = None;
                restored.diagnostic = Some(DiagnosticCode::ProviderFailure);
                if matches!(
                    claim.action,
                    ResourceAction::Create | ResourceAction::VerifyExternal
                ) {
                    restored.install = InstallProgress::Failed;
                }
                restored
            }
        };
        after.version = increment(before.version)?;
        execution_transitions::commit(
            &mut context,
            identity,
            Change {
                id,
                kind: "reconciled",
                operation_id: provenance.attempt_id.as_uuid(),
                fingerprint,
                claim,
                before: &before,
                after: &after,
                proof,
            },
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
