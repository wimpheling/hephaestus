use super::ReadyRetentionError;
use identity_domain::AuthenticatedIdentity;
use recipe_application::{OwnedReadyRetentionPlan, ResourceProgress, RetainOwnedReady};
use std::sync::Arc;
use volume_local::OwnedReadyRetentionCustody;
use volume_trait::{OwnedProvisioningClaim, OwnedReadyRetentionFact};

#[derive(Clone)]
pub(super) struct Request {
    pub identity: AuthenticatedIdentity,
    pub retention: RetainOwnedReady,
    pub claim: OwnedProvisioningClaim,
    pub scope: (String, String),
    pub handover: Option<super::handover::Comparison>,
}
impl Request {
    pub fn handover_from(&self, original: &Self) -> bool {
        self.handover.is_some()
            && self.claim == original.claim
            && self.scope == original.scope
            && self.retention.target == original.retention.target
            && self.retention.command != original.retention.command
            && self.retention.operation_id != original.retention.operation_id
    }
    pub fn exact(&self, current: &Self) -> bool {
        self.retention == current.retention
            && self.claim == current.claim
            && self.scope == current.scope
    }
}

/// The original tuple never moves out on an expected worker/actor error.
pub(super) struct Original {
    pub request: Request,
    pub active: Request,
    pub history: Vec<Request>,
    pub identities: Vec<AuthenticatedIdentity>,
    pub plan: Option<OwnedReadyRetentionPlan>,
    pub custody: Option<OwnedReadyRetentionCustody>,
    pub fact: Option<OwnedReadyRetentionFact>,
    pub committed: Option<ResourceProgress>,
    pub errors: Vec<Arc<ReadyRetentionError>>,
}
impl Original {
    pub fn new(request: Request) -> Self {
        Self {
            active: request.clone(),
            request,
            history: Vec::new(),
            identities: Vec::new(),
            plan: None,
            custody: None,
            fact: None,
            committed: None,
            errors: Vec::new(),
        }
    }

    pub fn plan(&mut self, plan: &OwnedReadyRetentionPlan) -> Result<(), ReadyRetentionError> {
        if let Some(original) = &self.plan {
            if original.preparation != plan.preparation
                || original.closure_id != plan.closure_id
                || original.vm_provider_namespace != plan.vm_provider_namespace
                || original.vm_host_id != plan.vm_host_id
            {
                return Err(ReadyRetentionError::Correlation);
            }
        }
        if let Some(committed) = &self.committed {
            if plan.completed.as_ref() != Some(committed) {
                return Err(ReadyRetentionError::Correlation);
            }
        }
        if self.plan.is_none() {
            self.plan = Some(plan.clone());
        }
        Ok(())
    }
}
