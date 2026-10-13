//! Native-qualified Pending publication retains exact inputs before commit await.
use super::super::{Context, Mode, PreparedInstanceDrainError, Request};
use super::{Admission, Original, retain_claim};
use capability_domain::CapabilitySlotKey;
use recipe_application::{
    ActiveDeploymentEffectRepository, AttemptProvenance, DeploymentError, DeploymentSnapshot,
    EffectClaim, OriginalInstanceClosure, PendingInstanceDrainClosure, PlatformPolicyObservation,
    PreparedInstanceClosureAdmission, PreparedInstanceClosureRepository,
    PreparedInstanceSafetyTarget, ResumedPreparedInstanceClosureRequest,
};

pub enum Endpoint {
    Initial,
    Existing(Box<OriginalInstanceClosure>),
    Pending(Box<PendingInstanceDrainClosure>),
}

pub fn select(
    closures: &[OriginalInstanceClosure],
    pending: &[PendingInstanceDrainClosure],
    resource: &CapabilitySlotKey,
) -> Result<Endpoint, DeploymentError> {
    let mut closed = closures
        .iter()
        .filter(|item| &item.drain.resource == resource);
    let closure = closed.next();
    let mut waiting = pending
        .iter()
        .filter(|item| &item.drain().resource == resource);
    let unpublished = waiting.next();
    if closed.next().is_some()
        || waiting.next().is_some()
        || (closure.is_some() && unpublished.is_some())
    {
        return Err(DeploymentError::ReconciliationRequired);
    }
    match (closure, unpublished) {
        (Some(closure), None) => Ok(Endpoint::Existing(Box::new(closure.clone()))),
        (None, Some(pending)) => Ok(Endpoint::Pending(Box::new(pending.clone()))),
        (None, None) => Ok(Endpoint::Initial),
        _ => Err(DeploymentError::ReconciliationRequired),
    }
}

/// Actual current request comparison data, never saved credentials to replay.
pub struct Publication {
    pub current: AttemptProvenance,
    pub request: ResumedPreparedInstanceClosureRequest,
}

pub fn plan(
    request: &Request,
    snapshot: &DeploymentSnapshot,
    target: &PreparedInstanceSafetyTarget,
    active: Option<&EffectClaim>,
    pending: &PendingInstanceDrainClosure,
) -> Result<ResumedPreparedInstanceClosureRequest, DeploymentError> {
    request.remove.command.validate(
        &request.identity,
        recipe_application::DeploymentOperation::Remove,
    )?;
    if request.mode != Mode::Resume
        || snapshot.intent.id() != request.remove.deployment_id
        || pending.snapshot() != snapshot
        || pending.current_command() != request.remove.command
        || pending.target() != target
        || pending.drain().resource != request.resource
        || active != Some(pending.drain())
        || snapshot.version != request.remove.expected_version
    {
        return Err(DeploymentError::IntentMismatch);
    }
    // This is the unpublished endpoint's own immutable FIRST admission version,
    // not today's CAS compared to another publisher's historical first version.
    ResumedPreparedInstanceClosureRequest::new(
        request.remove.command,
        snapshot.intent.clone(),
        target.clone(),
        pending.drain().clone(),
        pending.clone(),
        request.expected_instance_version,
    )
}

pub fn retain(
    original: &mut Original,
    identity: &identity_domain::AuthenticatedIdentity,
    request: ResumedPreparedInstanceClosureRequest,
) -> Result<ResumedPreparedInstanceClosureRequest, DeploymentError> {
    request
        .current_command()
        .validate(identity, recipe_application::DeploymentOperation::Remove)?;
    let current = AttemptProvenance::new(identity, request.drain().provenance.attempt_id)?;
    for operation in &original.resumed_publications {
        if operation.current == current
            && operation.request.current_command() == request.current_command()
        {
            if operation.request != request {
                return Err(DeploymentError::IntentMismatch);
            }
            return Ok(operation.request.clone());
        }
    }
    // A changed authenticated request appends. Earlier uncertain publication and
    // its held error remain owned; current credentials always come from this pass.
    original.resumed_publications.push(Publication {
        current,
        request: request.clone(),
    });
    Ok(request)
}

pub(super) async fn publish(
    context: &Context,
    request: &Request,
    original: &mut Original,
    snapshot: DeploymentSnapshot,
    target: PreparedInstanceSafetyTarget,
    active: Option<EffectClaim>,
    pending: PendingInstanceDrainClosure,
) -> Result<Admission, PreparedInstanceDrainError> {
    let configuration = &context.controller.drain_inputs().configuration;
    let scope = configuration.provider_scope();
    let platform = PlatformPolicyObservation::new(
        configuration.platform_policy().clone(),
        configuration.platform_version().to_owned(),
    )?;
    let expected_namespace = scope.namespace();
    if target.provider_namespace() != expected_namespace
        || target.host_id() != scope.host_id()
        || target.prepared().source().platform() != &platform
    {
        return Err(DeploymentError::IntentMismatch.into());
    }
    let planned = plan(request, &snapshot, &target, active.as_ref(), &pending)?;
    retain_claim(original, planned.drain(), &target)?;
    // Exact full original Create/Drain/snapshot/first-CAS/caller-CAS and current
    // publication identity are owned BEFORE the first closure transaction await.
    let retained = retain(original, &request.identity, planned)?;
    context.controller.validate_owners()?;
    let admitted = context
        .safety
        .close_resumed_for_remove(&request.identity, &retained)
        .await?;
    if admitted.target != target || admitted.removal_id != retained.removal_id() {
        return Err(DeploymentError::IntentMismatch.into());
    }
    // Read the genuine winning origin after commit, including a racing winner.
    // Never build publisher B credentials/metadata or infer success from absence.
    let resumed = context
        .deployment
        .admit_remove_resume(&request.identity, request.remove)
        .await?;
    if resumed.admission().snapshot.intent != snapshot.intent {
        return Err(DeploymentError::IntentMismatch.into());
    }
    let Endpoint::Existing(closure) = select(
        resumed.closures(),
        resumed.pending_instance_drains(),
        &request.resource,
    )?
    else {
        return Err(DeploymentError::ReconciliationRequired.into());
    };
    compare_winner(&admitted, &retained, &closure)?;
    let active = context
        .preparation
        .load_active_effect(
            &request.identity,
            request.remove.command,
            request.remove.deployment_id,
            &request.resource,
        )
        .await?;
    super::existing_qualified(
        context,
        request,
        original,
        snapshot.intent,
        target,
        active,
        *closure,
    )
    .await
}

// Comparison only; production calls this AFTER actual native publication/winner
// and a fresh protected admission reload. Caller-created values grant nothing.
pub fn compare_winner(
    admitted: &PreparedInstanceClosureAdmission,
    request: &ResumedPreparedInstanceClosureRequest,
    closure: &OriginalInstanceClosure,
) -> Result<(), DeploymentError> {
    if admitted.target != *request.target()
        || admitted.removal_id != request.removal_id()
        || closure.drain != *request.drain()
        || closure.removal_id != admitted.removal_id
        || closure.command_key != request.command_key()
        || (!admitted.replayed && closure.expected_version != request.expected_instance_version())
    {
        return Err(DeploymentError::IntentMismatch);
    }
    // A genuine positive winner retains its stored CAS, not today's caller CAS.
    // The next shared helper still requalifies that full saved closure under C.
    Ok(())
}

// Production passes the actual protected current-identity loader result. This
// pure comparison is not itself a grant or a physical cleanup certificate.
pub fn compare_loaded(
    expected: &OriginalInstanceClosure,
    actual: Result<OriginalInstanceClosure, DeploymentError>,
) -> Result<(), DeploymentError> {
    if actual? != *expected {
        return Err(DeploymentError::ReconciliationRequired);
    }
    Ok(())
}
