use recipe_application::{
    DeploymentAttemptId, DeploymentError, DeploymentOperation, DiagnosticCode, EffectClaim,
    EffectEvidence, EffectOutcome, InstallProgress, PlannedResource, RemovalProgress,
    ResourceAction, ResourceOwnership, ResourceProgress,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub install: InstallProgress,
    pub removal: RemovalProgress,
    pub version: i64,
    pub active_attempt: Option<uuid::Uuid>,
    pub diagnostic: Option<DiagnosticCode>,
    pub generation: i64,
    pub last_action: Option<ResourceAction>,
}

impl State {
    pub fn progress(&self) -> Result<ResourceProgress, DeploymentError> {
        Ok(ResourceProgress {
            install: self.install,
            removal: self.removal,
            version: crate::hydration::unsigned(self.version)?,
            active_attempt: self
                .active_attempt
                .map(DeploymentAttemptId::from_uuid)
                .transpose()?,
            diagnostic: self.diagnostic,
        })
    }

    pub fn claim(&self, claim: &EffectClaim) -> Result<Self, DeploymentError> {
        let mut next = self.clone();
        next.version = self
            .version
            .checked_add(1)
            .ok_or(DeploymentError::StaleClaim)?;
        next.generation = self
            .generation
            .checked_add(1)
            .ok_or(DeploymentError::StaleClaim)?;
        next.active_attempt = Some(claim.provenance.attempt_id.as_uuid());
        next.diagnostic = None;
        match claim.action {
            ResourceAction::Create | ResourceAction::VerifyExternal => {
                next.install = InstallProgress::Creating;
            }
            ResourceAction::Drain | ResourceAction::Detach => {
                next.removal = RemovalProgress::Draining;
            }
            ResourceAction::Retain | ResourceAction::Delete => {}
        }
        Ok(next)
    }

    pub fn applied(&self, action: ResourceAction) -> Self {
        let mut next = self.clone();
        next.active_attempt = None;
        next.diagnostic = None;
        next.last_action = Some(action);
        match action {
            ResourceAction::Create | ResourceAction::VerifyExternal => {
                next.install = InstallProgress::Ready;
            }
            ResourceAction::Drain => next.removal = RemovalProgress::Draining,
            ResourceAction::Detach => next.removal = RemovalProgress::Detached,
            ResourceAction::Retain => next.removal = RemovalProgress::Retained,
            ResourceAction::Delete => next.removal = RemovalProgress::Deleted,
        }
        next
    }

    pub fn uncertain(&self, action: ResourceAction, diagnostic: DiagnosticCode) -> Self {
        let mut next = self.clone();
        next.diagnostic = Some(diagnostic);
        match action {
            ResourceAction::Create | ResourceAction::VerifyExternal => {
                next.install = InstallProgress::RecoveryRequired;
            }
            _ => next.removal = RemovalProgress::RecoveryRequired,
        }
        next
    }
}

pub fn legality(
    plan: &PlannedResource,
    state: &State,
    operation: DeploymentOperation,
    action: ResourceAction,
) -> Result<(), DeploymentError> {
    plan.validate_action(action)?;
    if state.active_attempt.is_some() {
        return Err(DeploymentError::ReconciliationRequired);
    }
    let allowed = match (operation, action) {
        (DeploymentOperation::Install, ResourceAction::Create | ResourceAction::VerifyExternal) => {
            matches!(
                state.install,
                InstallProgress::Pending | InstallProgress::Failed
            )
        }
        (DeploymentOperation::Remove, ResourceAction::Drain) => {
            state.removal == RemovalProgress::Pending
        }
        (DeploymentOperation::Remove, ResourceAction::Detach) => match plan.identity() {
            recipe_application::PlannedResourceIdentity::Volume { .. } => {
                state.removal == RemovalProgress::Pending
            }
            recipe_application::PlannedResourceIdentity::Instance { .. } => {
                state.removal == RemovalProgress::Draining
                    && state.last_action == Some(ResourceAction::Drain)
            }
        },
        (DeploymentOperation::Remove, ResourceAction::Retain | ResourceAction::Delete) => {
            state.removal == RemovalProgress::Detached
                && state.last_action == Some(ResourceAction::Detach)
        }
        _ => false,
    };
    if allowed
        && (operation != DeploymentOperation::Remove
            || plan.ownership() == ResourceOwnership::Owned)
    {
        Ok(())
    } else {
        Err(DeploymentError::InvalidAction)
    }
}

pub const fn outcome(evidence: EffectEvidence) -> (&'static str, Option<DiagnosticCode>) {
    match evidence.outcome {
        EffectOutcome::Applied => ("applied", None),
        EffectOutcome::Failed(code) => ("failed", Some(code)),
    }
}
