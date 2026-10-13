use super::{active, exact, increment, reconciliation_scope};
use crate::{
    PostgresDeploymentRepository,
    effect_context::Context,
    effect_state::{self, State},
    execution_transitions::{self, Change},
    verification,
};
use identity_domain::AuthenticatedIdentity;
use recipe_application::{
    AttemptProvenance, CommandIdentity, DeploymentError, DiagnosticCode, EffectClaim,
    EffectEvidence, EffectOutcome, InstallProgress, ReconciledOutcome, ResourceAction,
};
use serde_json::json;

pub async fn complete(
    context: &mut Context<'_>,
    identity: &AuthenticatedIdentity,
    claim: &EffectClaim,
    evidence: EffectEvidence,
) -> Result<State, DeploymentError> {
    exact(&mut context.tx, claim).await?;
    if evidence.identity != claim.identity || evidence.input_hash != claim.input_hash {
        return Err(DeploymentError::InputConflict);
    }
    let (kind, diagnostic) = effect_state::outcome(evidence);
    let fingerprint =
        execution_transitions::fingerprint(claim, "completed", &json!([kind, diagnostic]))?;
    let id =
        execution_transitions::transition_id(claim.provenance.attempt_id.as_uuid(), "completed");
    if let Some(state) = execution_transitions::replay(&mut context.tx, id, fingerprint).await? {
        return Ok(state);
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
        context,
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
    Ok(after)
}

pub async fn reconcile(
    repository: &PostgresDeploymentRepository,
    context: &mut Context<'_>,
    identity: &AuthenticatedIdentity,
    command: CommandIdentity,
    claim: &EffectClaim,
    provenance: AttemptProvenance,
    outcome: ReconciledOutcome,
) -> Result<State, DeploymentError> {
    let original = exact(&mut context.tx, claim).await?;
    if reconciliation_scope(context, command, claim).is_err()
        && !crate::remove_resume::selected(
            &mut context.tx,
            command,
            claim,
            repository.remove_resume.as_ref(),
        )
        .await?
    {
        return Err(DeploymentError::InvalidAction);
    }
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
    let id = execution_transitions::transition_id(provenance.attempt_id.as_uuid(), "reconciled");
    if let Some(state) = execution_transitions::replay(&mut context.tx, id, fingerprint).await? {
        return Ok(state);
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
        context,
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
    Ok(after)
}
