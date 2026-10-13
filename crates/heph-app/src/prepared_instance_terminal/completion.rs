//! Exact current-authenticated completion identity is owned before worker await.
use super::{Context, PreparedInstanceTerminalError, Request, retained::Original};
use recipe_application::{
    ActiveDeploymentEffectRepository, AttemptProvenance, DeploymentAttemptId, DeploymentError,
    EffectClaim, EffectEvidence, EffectOutcome, ReconciledOutcome, RemovalProgress, ResourceAction,
    ResourceProgress,
};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Completion {
    Original,
    Reconcile(AttemptProvenance),
}
impl Completion {
    const fn attempt(self, claim: &EffectClaim) -> DeploymentAttemptId {
        match self {
            Self::Original => claim.provenance.attempt_id,
            Self::Reconcile(p) => p.attempt_id,
        }
    }
}
#[derive(PartialEq, Eq)]
struct Key {
    remove: recipe_application::RemoveDeployment,
    current: AttemptProvenance,
    claim: EffectClaim,
}
pub(super) struct Operation {
    key: Key,
    pub completion: Completion,
    pub verification: Option<Uuid>,
    pub progress: Option<ResourceProgress>,
}
pub(super) fn choose(
    request: &Request,
    claim: &EffectClaim,
    progress: &ResourceProgress,
    original: &mut Original,
) -> Result<usize, DeploymentError> {
    if progress.version < claim.resource_version
        || progress.active_attempt != Some(claim.provenance.attempt_id)
        || !original.claims.contains(claim)
    {
        return Err(DeploymentError::ReconciliationRequired);
    }
    request.remove.command.validate(
        &request.identity,
        recipe_application::DeploymentOperation::Remove,
    )?;
    let current = AttemptProvenance::new(&request.identity, claim.provenance.attempt_id)?;
    let is_original = request.remove.command == claim.command
        && current == claim.provenance
        && progress.version == claim.resource_version;
    let key = Key {
        remove: request.remove,
        current,
        claim: claim.clone(),
    };
    if let Some(i) = original.completions.iter().position(|old| {
        old.key == key && matches!(old.completion, Completion::Original) == is_original
    }) {
        return Ok(i);
    }
    let completion = if is_original {
        Completion::Original
    } else {
        Completion::Reconcile(AttemptProvenance::new(
            &request.identity,
            DeploymentAttemptId::from_uuid(Uuid::new_v4())?,
        )?)
    };
    original.completions.push(Operation {
        key,
        completion,
        verification: None,
        progress: None,
    });
    Ok(original.completions.len() - 1)
}
pub(super) async fn commit(
    context: &Context,
    request: &Request,
    original: &mut Original,
    claim: &EffectClaim,
) -> Result<ResourceProgress, PreparedInstanceTerminalError> {
    let active = context
        .preparation
        .load_active_effect(
            &request.identity,
            request.remove.command,
            request.remove.deployment_id,
            &request.resource,
        )
        .await?;
    if active.as_ref() != Some(claim) {
        return Err(DeploymentError::ReconciliationRequired.into());
    }
    let snapshot = context
        .deployment
        .inspect(&request.identity, request.remove.deployment_id)
        .await?;
    let progress = snapshot
        .resources
        .get(&request.resource)
        .ok_or(DeploymentError::Unavailable)?;
    let i = choose(request, claim, progress, original)?;
    let completion = original.completions[i].completion;
    context.controller.validate_owners()?;
    let verification = context
        .recorder
        .record(
            claim,
            completion.attempt(claim),
            recipe_postgres::VerifiedOutcome::Applied,
        )
        .await?;
    if original.completions[i]
        .verification
        .is_some_and(|old| old != verification)
    {
        return Err(DeploymentError::InputConflict.into());
    }
    original.completions[i].verification = Some(verification);
    context.controller.validate_owners()?;
    let evidence = EffectEvidence {
        identity: claim.identity,
        input_hash: claim.input_hash,
        outcome: EffectOutcome::Applied,
    };
    let progress = match completion {
        Completion::Original => {
            context
                .deployment
                .complete_resource_effect(&request.identity, claim, evidence)
                .await?
        }
        Completion::Reconcile(provenance) => {
            context
                .deployment
                .record_reconciliation(
                    &request.identity,
                    request.remove.command,
                    claim,
                    provenance,
                    ReconciledOutcome::Applied(evidence),
                )
                .await?
        }
    };
    let expected = match claim.action {
        ResourceAction::Detach => RemovalProgress::Detached,
        ResourceAction::Delete => RemovalProgress::Deleted,
        _ => return Err(DeploymentError::InvalidAction.into()),
    };
    if progress.active_attempt.is_some()
        || progress.removal != expected
        || progress.version <= claim.resource_version
    {
        return Err(DeploymentError::IntentMismatch.into());
    }
    original.completions[i].progress = Some(progress.clone());
    Ok(progress)
}
