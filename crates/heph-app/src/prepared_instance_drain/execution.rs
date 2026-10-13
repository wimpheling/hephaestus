//! Whole-set comparison, actual original cancellation, then genuine worker/actor commit.

use super::{
    Context, PreparedInstanceDrainError, Request,
    admission::{self, Admission, Original},
};
use recipe_application::{
    ActiveDeploymentEffectRepository, AttemptProvenance, DeploymentAttemptId, DeploymentError,
    EffectClaim, EffectEvidence, EffectOutcome, PreparedInstanceSafetyObservation,
    PreparedInstanceSafetyRepository, PreparedInstanceSafetyState, ReconciledOutcome,
    RemoveDeployment, ResourceProgress,
};
use run_postgres::InstanceDrainOriginal;
use uuid::Uuid;

pub(super) async fn drain(
    context: &Context,
    request: &Request,
    original: &mut Original,
) -> Result<ResourceProgress, PreparedInstanceDrainError> {
    retain_completion_error(original);
    let (target, claim, removal) =
        match Box::pin(admission::prepare(context, request, original)).await? {
            Admission::Completed(progress) => return Ok(progress), // Durable proof, no native IO/receipt/claim.
            Admission::Active(plan) => {
                let admission::Active {
                    target,
                    claim,
                    removal,
                } = *plan;
                (target, claim, removal)
            }
        };
    let drain_claim = recipe_postgres::encode_effect_claim(&claim)?;
    let create_claim = recipe_postgres::encode_effect_claim(target.original())?;
    let prepared =
        serde_json::to_value(target.prepared()).map_err(|_| DeploymentError::Serialization)?;
    let rows = context
        .reader
        .observe(&InstanceDrainOriginal {
            instance_id: target.prepared().instance_id(),
            drain_attempt: claim.provenance.attempt_id.as_uuid(),
            drain_claim: &drain_claim,
            create_attempt: target.original().provenance.attempt_id.as_uuid(),
            create_claim: &create_claim,
            prepared: &prepared,
        })
        .await
        .map_err(run_orchestrator::OrchestratorError::from)?;
    // The native wrapper qualified the entire actual immutable DTO inventory.
    // This SAME retained controller qualifies every row BEFORE the first delivery.
    deliver_after_qualification(
        async {
            context
                .controller
                .qualify_instance_drain_batch(&target, &rows)
                .await
                .map_err(Into::into)
        },
        &rows,
        |row| async move {
            if let Some(cancel) = &row.cancel {
                context.controller.clone().cancel_run(cancel).await?;
            }
            // None never becomes a synthetic command or cleanup receipt.
            Ok(())
        },
    )
    .await?;
    let observed = context.safety.observe_safety(&target).await?;
    require_drained(&observed, &target, removal)?;
    let active = context
        .preparation
        .load_active_effect(
            &request.identity,
            request.remove.command,
            request.remove.deployment_id,
            &request.resource,
        )
        .await?;
    if active.as_ref() != Some(&claim) {
        // A racing completed actor is not inferred from missing active progress.
        // Fresh distinct manager retry uses the durable completed observer instead.
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
    let completion = completion(request, &claim, progress, original)?;
    context.controller.validate_owners()?; // Original roots and provider BEFORE genuine receipt.
    context
        .recorder
        .record(
            &claim,
            completion.attempt(&claim),
            recipe_postgres::VerifiedOutcome::Applied,
        )
        .await?;
    complete_after_worker(context, request, &claim, completion).await
}

async fn complete_after_worker(
    context: &Context,
    request: &Request,
    claim: &EffectClaim,
    completion: Completion,
) -> Result<ResourceProgress, PreparedInstanceDrainError> {
    let evidence = EffectEvidence {
        identity: claim.identity,
        input_hash: claim.input_hash,
        outcome: EffectOutcome::Applied,
    };
    match completion {
        Completion::Original => context
            .deployment
            .complete_resource_effect(&request.identity, claim, evidence)
            .await
            .map_err(Into::into),
        Completion::Reconcile(provenance) => context
            .deployment
            .record_reconciliation(
                &request.identity,
                request.remove.command,
                claim,
                provenance,
                ReconciledOutcome::Applied(evidence),
            )
            .await
            .map_err(Into::into),
    }
    // Actor transaction rechecks current rights/fence/proof. No Delete or Detach follows.
}

pub(super) fn require_drained(
    observed: &PreparedInstanceSafetyObservation,
    target: &recipe_application::PreparedInstanceSafetyTarget,
    removal: release_domain::InstanceRemovalId,
) -> Result<(), PreparedInstanceDrainError> {
    if &observed.target != target || observed.removal_id != Some(removal) {
        return Err(DeploymentError::IntentMismatch.into());
    }
    if observed.state != PreparedInstanceSafetyState::Drained {
        return Err(PreparedInstanceDrainError::Pending);
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Completion {
    Original,
    Reconcile(AttemptProvenance),
}
impl Completion {
    const fn attempt(&self, claim: &EffectClaim) -> DeploymentAttemptId {
        match self {
            Self::Original => claim.provenance.attempt_id,
            Self::Reconcile(provenance) => provenance.attempt_id,
        }
    }
}
/// Retained comparison identity, never authorization or provider proof.
#[derive(PartialEq, Eq)]
struct CompletionKey {
    remove: RemoveDeployment,
    current: AttemptProvenance,
    claim: EffectClaim,
}

pub(super) struct CompletionOperation {
    key: CompletionKey,
    pub completion: Completion,
}

/// Preserve actual previous held errors alongside every retained operation.
/// The error describes an owning pass; it is not a verification receipt.
pub(super) fn retain_completion_error(original: &mut Original) {
    let Some(error) = &original.last_error else {
        return;
    };
    if !original
        .completion_errors
        .iter()
        .any(|held| std::sync::Arc::ptr_eq(held, error))
    {
        original.completion_errors.push(error.clone());
    }
}

pub(super) fn completion(
    request: &Request,
    claim: &EffectClaim,
    progress: &ResourceProgress,
    original: &mut Original,
) -> Result<Completion, DeploymentError> {
    if progress.version < claim.resource_version {
        return Err(DeploymentError::StaleClaim);
    }
    if progress.active_attempt != Some(claim.provenance.attempt_id) {
        return Err(DeploymentError::ReconciliationRequired);
    }
    if original.claim.as_ref() != Some(claim) {
        return Err(DeploymentError::InputConflict);
    }
    request.remove.command.validate(
        &request.identity,
        recipe_application::DeploymentOperation::Remove,
    )?;
    let current = AttemptProvenance::new(&request.identity, claim.provenance.attempt_id)?;
    let is_original = request.remove.command == claim.command
        && progress.version == claim.resource_version
        && current == claim.provenance;
    let key = CompletionKey {
        remove: request.remove,
        current,
        claim: claim.clone(),
    };
    if let Some(previous) = original.completions.iter().find(|operation| {
        operation.key == key && matches!(operation.completion, Completion::Original) == is_original
    }) {
        return Ok(previous.completion);
    }
    let completion = if is_original {
        Completion::Original
    } else {
        Completion::Reconcile(AttemptProvenance::new(
            &request.identity,
            DeploymentAttemptId::from_uuid(Uuid::new_v4())?,
        )?)
    };
    // This synchronous retention precedes both worker recording and actor commit.
    // Changed authenticated keys append; uncertain original operations stay owned.
    original
        .completions
        .push(CompletionOperation { key, completion });
    Ok(completion)
}

/// A local scheduling primitive, not a qualification or execution port.
pub(super) async fn deliver_after_qualification<'a, F, Work>(
    qualified: impl std::future::Future<Output = Result<(), PreparedInstanceDrainError>>,
    rows: &'a [run_postgres::InstanceDrainRunObservation],
    mut deliver: F,
) -> Result<(), PreparedInstanceDrainError>
where
    F: FnMut(&'a run_postgres::InstanceDrainRunObservation) -> Work,
    Work: std::future::Future<Output = Result<(), PreparedInstanceDrainError>>,
{
    qualified.await?;
    for row in rows {
        deliver(row).await?;
    }
    Ok(())
}
