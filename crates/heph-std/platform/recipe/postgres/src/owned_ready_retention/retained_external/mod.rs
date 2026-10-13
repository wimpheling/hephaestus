//! Explicit retained reuse: reopening and normal Applied completion share one actor transaction.
mod origin;
mod paired;

use super::{PostgresOwnedReadyRetentionRepository, reopening};
use crate::{
    effect_context::Context, effect_outcome, effect_state::State, execution_rows::ClaimWire,
    repository_error, verification,
};
use identity_domain::AuthenticatedIdentity;
use recipe_application::{
    AttemptProvenance, DeploymentError, DeploymentOperation, EffectClaim, EffectEvidence,
    EffectOutcome, PlannedResourceIdentity, ReconciledOutcome, ResourceAction, ResourceProgress,
    RetainedExternalCompletion, RetainedExternalVerificationPlan,
};
use runtime_types::VolumeId;

pub(super) async fn prepare(
    adapter: &PostgresOwnedReadyRetentionRepository,
    identity: &AuthenticatedIdentity,
    claim: &EffectClaim,
) -> Result<RetainedExternalVerificationPlan, DeploymentError> {
    let id = validate(identity, claim)?;
    let (mut context, state) = reopening::current_context(adapter, identity, claim, id).await?;
    let mut plan = origin::load(adapter, &mut context, claim).await?;
    if let Some(completed) = paired::load(&mut context, &state, claim, plan.closure_id).await? {
        plan.completed = Some(completed.progress);
    } else {
        pending(&mut context, &state, &plan).await?;
    }
    context.tx.commit().await.map_err(repository_error)?;
    Ok(plan)
}

pub(super) async fn complete(
    adapter: &PostgresOwnedReadyRetentionRepository,
    identity: &AuthenticatedIdentity,
    plan: &RetainedExternalVerificationPlan,
    completion: RetainedExternalCompletion,
) -> Result<ResourceProgress, DeploymentError> {
    let claim = &plan.claim;
    let id = validate(identity, claim)?;
    let verification_attempt = match completion {
        RetainedExternalCompletion::Completed => claim.provenance.attempt_id,
        RetainedExternalCompletion::Reconciled(provenance) => {
            if provenance != AttemptProvenance::new(identity, provenance.attempt_id)?
                || provenance.attempt_id == claim.provenance.attempt_id
            {
                return Err(DeploymentError::InputConflict);
            }
            provenance.attempt_id
        }
    };
    let (mut context, state) = reopening::current_context(adapter, identity, claim, id).await?;
    let current = origin::load(adapter, &mut context, claim).await?;
    exact_plan(plan, &current)?;
    if let Some(completed) = paired::load(&mut context, &state, claim, current.closure_id).await? {
        completed.matches(claim, completion)?;
        if plan
            .completed
            .as_ref()
            .is_some_and(|expected| expected != &completed.progress)
        {
            return Err(DeploymentError::InputConflict);
        }
        context.tx.commit().await.map_err(repository_error)?;
        return Ok(completed.progress);
    }
    if plan.completed.is_some() {
        return Err(DeploymentError::ReconciliationRequired);
    }
    pending(&mut context, &state, &current).await?;
    // Only a genuine trusted worker receipt can permit the transaction's reopening.
    verification::matching(
        &mut context.tx,
        claim,
        verification_attempt,
        "applied",
        None,
    )
    .await?;
    let json =
        serde_json::to_value(ClaimWire::new(claim)?).map_err(|_| DeploymentError::Serialization)?;
    reopening::append(
        &mut context,
        identity,
        claim,
        current.closure_id,
        paired::reopening_id(claim),
        json,
    )
    .await?;
    let evidence = EffectEvidence {
        identity: claim.identity,
        input_hash: claim.input_hash,
        outcome: EffectOutcome::Applied,
    };
    let after = match completion {
        RetainedExternalCompletion::Completed => {
            effect_outcome::complete(&mut context, identity, claim, evidence).await?
        }
        RetainedExternalCompletion::Reconciled(provenance) => {
            effect_outcome::reconcile(
                &adapter.repository,
                &mut context,
                identity,
                claim.command,
                claim,
                provenance,
                ReconciledOutcome::Applied(evidence),
            )
            .await?
        }
    };
    context.tx.commit().await.map_err(repository_error)?;
    after.progress()
}

fn validate(
    identity: &AuthenticatedIdentity,
    claim: &EffectClaim,
) -> Result<VolumeId, DeploymentError> {
    claim
        .command
        .validate(identity, DeploymentOperation::Install)?;
    if claim.action != ResourceAction::VerifyExternal {
        return Err(DeploymentError::InvalidAction);
    }
    let PlannedResourceIdentity::Volume { id, .. } = claim.identity else {
        return Err(DeploymentError::InvalidAction);
    };
    Ok(id)
}

async fn pending(
    context: &mut Context<'_>,
    state: &State,
    plan: &RetainedExternalVerificationPlan,
) -> Result<(), DeploymentError> {
    if state.active_attempt != Some(plan.claim.provenance.attempt_id.as_uuid()) {
        return Err(DeploymentError::StaleClaim);
    }
    let quiescent: bool = sqlx::query_scalar("SELECT public.ready_retention_quiescent($1) IS TRUE")
        .bind(plan.closure_id)
        .fetch_one(&mut *context.tx)
        .await
        .map_err(repository_error)?;
    if !quiescent {
        return Err(DeploymentError::ReconciliationRequired);
    }
    Ok(())
}

fn exact_plan(
    expected: &RetainedExternalVerificationPlan,
    current: &RetainedExternalVerificationPlan,
) -> Result<(), DeploymentError> {
    let mut expected = expected.clone();
    expected.completed = None;
    if expected != *current {
        return Err(DeploymentError::InputConflict);
    }
    Ok(())
}
