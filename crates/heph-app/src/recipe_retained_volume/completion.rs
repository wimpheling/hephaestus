use identity_domain::AuthenticatedIdentity;
use recipe_application::{
    AttemptProvenance, DeploymentAttemptId, DeploymentError, DiagnosticCode, ResourceProgress,
    RetainedExternalCompletion, RetainedExternalVerificationPlan,
};
use recipe_postgres::VerifiedOutcome;
use uuid::Uuid;
use volume_local::OwnedRetainedReuseCustody;
use volume_trait::VolumeError;

use super::{
    PreparedRetainedVolumeVerifier, RetainedVolumeVerificationError,
    RetainedVolumeVerificationFailure, validation,
};

pub struct Resolution {
    pub result: Result<ResourceProgress, RetainedVolumeVerificationError>,
    pub resolved: bool,
}

pub async fn commit(
    service: &PreparedRetainedVolumeVerifier,
    identity: &AuthenticatedIdentity,
    plan: &RetainedExternalVerificationPlan,
    custody: &mut OwnedRetainedReuseCustody,
) -> Resolution {
    match finish(service, identity, plan, custody).await {
        Ok(progress) => Resolution {
            result: Ok(progress),
            resolved: true,
        },
        Err(failure) => recover(service, identity, plan, failure).await,
    }
}

async fn finish(
    service: &PreparedRetainedVolumeVerifier,
    identity: &AuthenticatedIdentity,
    plan: &RetainedExternalVerificationPlan,
    custody: &mut OwnedRetainedReuseCustody,
) -> Result<ResourceProgress, RetainedVolumeVerificationFailure> {
    let current = fresh(service, identity, plan).await?;
    if let Some(progress) = current.completed {
        return Ok(progress); // Never physically revalidate a committed reopened pair.
    }
    custody.revalidate()?;
    let held = custody.context();
    let current = service
        .worker
        .retained_reuse_context(
            &held.retained.original.claim,
            held.retained.closure.fields().id,
            held.retention_operation_id,
        )
        .await?;
    if current != *custody.context() {
        return Err(VolumeError::IntentConflict.into());
    }
    let current = fresh(service, identity, plan).await?;
    if let Some(progress) = current.completed {
        return Ok(progress);
    }
    let completion = completion_attempt(service, identity, plan).await?;
    let attempt = match completion {
        RetainedExternalCompletion::Completed => plan.claim.provenance.attempt_id,
        RetainedExternalCompletion::Reconciled(provenance) => provenance.attempt_id,
    };
    custody.revalidate()?; // Actual physical proof immediately precedes genuine Applied.
    service
        .recorder
        .record(&plan.claim, attempt, VerifiedOutcome::Applied)
        .await?;
    let current = fresh(service, identity, plan).await?;
    if let Some(progress) = current.completed {
        return Ok(progress);
    }
    custody.revalidate()?;
    service
        .actor
        .complete_retained_external_verification(identity, plan, completion)
        .await
        .map_err(Into::into)
    // The actor transaction pairs reopening with Applied and rechecks current
    // authority. After commit runtime admission can open: no postcommit fsck.
}

async fn completion_attempt(
    service: &PreparedRetainedVolumeVerifier,
    identity: &AuthenticatedIdentity,
    plan: &RetainedExternalVerificationPlan,
) -> Result<RetainedExternalCompletion, DeploymentError> {
    let snapshot = service
        .deployment
        .inspect(identity, plan.claim.deployment_id)
        .await?;
    let progress = snapshot
        .resources
        .get(&plan.claim.resource)
        .ok_or(DeploymentError::Unavailable)?;
    if progress.active_attempt != Some(plan.claim.provenance.attempt_id) {
        return Err(DeploymentError::ReconciliationRequired);
    }
    if progress.version == plan.claim.resource_version {
        return Ok(RetainedExternalCompletion::Completed);
    }
    let attempt = DeploymentAttemptId::from_uuid(Uuid::new_v4())?;
    Ok(RetainedExternalCompletion::Reconciled(
        AttemptProvenance::new(identity, attempt)?,
    ))
}

async fn fresh(
    service: &PreparedRetainedVolumeVerifier,
    identity: &AuthenticatedIdentity,
    plan: &RetainedExternalVerificationPlan,
) -> Result<RetainedExternalVerificationPlan, DeploymentError> {
    let current = service
        .actor
        .prepare_retained_external_verification(identity, &plan.claim)
        .await?;
    validation::plan(service.provider.as_ref(), &current, &plan.claim)?;
    validation::unchanged(plan, &current)?;
    Ok(current)
}

async fn recover(
    service: &PreparedRetainedVolumeVerifier,
    identity: &AuthenticatedIdentity,
    plan: &RetainedExternalVerificationPlan,
    failure: RetainedVolumeVerificationFailure,
) -> Resolution {
    // A commit error may mean the complete pair committed. Only this exact pair
    // reread with actual current request authorization can resolve that outcome.
    let recovery_error = match fresh(service, identity, plan).await {
        Ok(current) => {
            if let Some(progress) = current.completed {
                return Resolution {
                    result: Ok(progress),
                    resolved: true,
                };
            }
            None
        }
        Err(error) => Some(error),
    };
    let ambiguity_error = service
        .deployment
        .mark_resource_ambiguous(
            identity,
            &plan.claim,
            DiagnosticCode::ProviderOutcomeUnknown,
        )
        .await
        .err();
    let resolved = recovery_error.is_none();
    Resolution {
        result: Err(RetainedVolumeVerificationError::Held {
            failure,
            recovery_error,
            ambiguity_error,
            custody_retained: !resolved,
        }),
        resolved,
    }
}

pub async fn resolve_pending(
    service: &PreparedRetainedVolumeVerifier,
    identity: &AuthenticatedIdentity,
    plan: &RetainedExternalVerificationPlan,
) -> Resolution {
    let recovery_error = match fresh(service, identity, plan).await {
        Ok(current) => {
            if let Some(progress) = current.completed {
                return Resolution {
                    result: Ok(progress),
                    resolved: true,
                };
            }
            None // Successful qualified closed preparation proves no atomic pair committed.
        }
        Err(error) => Some(error),
    };
    let ambiguity_error = service
        .deployment
        .mark_resource_ambiguous(
            identity,
            &plan.claim,
            DiagnosticCode::ProviderOutcomeUnknown,
        )
        .await
        .err();
    recovery_error.map_or_else(
        || Resolution {
            result: Err(DeploymentError::ReconciliationRequired.into()),
            resolved: true,
        },
        |recovery_error| Resolution {
            result: Err(RetainedVolumeVerificationError::Pending {
                recovery_error,
                ambiguity_error,
            }),
            resolved: false,
        },
    )
}
