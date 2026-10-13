//! Exact original comparison data remains owned across uncertain awaits.
use super::{PreparedInstanceTerminalError, Request, completion};
use recipe_application::{
    DeploymentError, DeploymentIntent, EffectClaim, PreparedInstanceDeleteCompletedHistory,
    PreparedInstanceDeleteHistory, PreparedInstanceDeleteHistoryState,
    PreparedInstanceDeleteObservation, PreparedInstanceDeletePublication,
    PreparedInstanceDeleteRequest, PreparedInstanceDetachObservation,
    PreparedInstanceDetachRequest, PreparedInstanceSafetyTarget, ResourceEffect,
};
use std::sync::Arc;

pub(super) struct Original {
    pub request: Request,
    pub intent: Option<DeploymentIntent>,
    pub target: Option<PreparedInstanceSafetyTarget>,
    pub proposals: Vec<ResourceEffect>,
    pub claims: Vec<EffectClaim>,
    pub detach: Vec<PreparedInstanceDetachRequest>,
    pub delete: Vec<PreparedInstanceDeleteRequest>,
    pub histories: Vec<PreparedInstanceDeleteHistory>,
    pub completed: Vec<PreparedInstanceDeleteCompletedHistory>,
    pub detach_observations: Vec<PreparedInstanceDetachObservation>,
    pub publications: Vec<PreparedInstanceDeleteObservation>,
    pub worker_observations: Vec<PreparedInstanceDeleteObservation>,
    pub completions: Vec<completion::Operation>,
    pub errors: Vec<Arc<PreparedInstanceTerminalError>>,
}
impl Original {
    pub const fn new(request: Request) -> Self {
        Self {
            request,
            intent: None,
            target: None,
            proposals: Vec::new(),
            claims: Vec::new(),
            detach: Vec::new(),
            delete: Vec::new(),
            histories: Vec::new(),
            completed: Vec::new(),
            detach_observations: Vec::new(),
            publications: Vec::new(),
            worker_observations: Vec::new(),
            completions: Vec::new(),
            errors: Vec::new(),
        }
    }
    pub fn target(&mut self, target: &PreparedInstanceSafetyTarget) -> Result<(), DeploymentError> {
        if self.target.as_ref().is_some_and(|old| old != target) {
            return Err(DeploymentError::IntentMismatch);
        }
        self.target = Some(target.clone());
        Ok(())
    }
    pub fn claim(&mut self, claim: &EffectClaim) -> Result<(), DeploymentError> {
        if self
            .claims
            .iter()
            .any(|old| old.action == claim.action && old != claim)
        {
            return Err(DeploymentError::InputConflict);
        }
        if !self.claims.contains(claim) {
            self.claims.push(claim.clone());
        }
        Ok(())
    }
    pub fn detach(
        &mut self,
        request: &PreparedInstanceDetachRequest,
    ) -> Result<(), DeploymentError> {
        if self.detach.iter().any(|old| {
            old.intent() != request.intent()
                || old.target() != request.target()
                || old.closure() != request.closure()
                || old.effect() != request.effect()
                || old.configuration() != request.configuration()
        }) {
            return Err(DeploymentError::IntentMismatch);
        }
        if !self.detach.contains(request) {
            self.detach.push(request.clone());
        }
        Ok(())
    }
    pub fn delete(
        &mut self,
        request: &PreparedInstanceDeleteRequest,
    ) -> Result<(), DeploymentError> {
        if self.delete.iter().any(|old| !same_delete(old, request)) {
            return Err(DeploymentError::IntentMismatch);
        }
        if !self.delete.contains(request) {
            self.delete.push(request.clone());
        }
        Ok(())
    }
    pub fn history(
        &mut self,
        history: &PreparedInstanceDeleteHistory,
    ) -> Result<(), DeploymentError> {
        for old in &self.histories {
            if !same_delete(old.original().original(), history.original().original())
                || (old.state() == PreparedInstanceDeleteHistoryState::Applied
                    && history.state() != PreparedInstanceDeleteHistoryState::Applied)
            {
                return Err(DeploymentError::IntentMismatch);
            }
            match (
                old.original().publication(),
                history.original().publication(),
            ) {
                (
                    PreparedInstanceDeletePublication::Committed(old),
                    PreparedInstanceDeletePublication::Committed(new),
                ) if old != new => return Err(DeploymentError::IntentMismatch),
                (
                    PreparedInstanceDeletePublication::Committed(_),
                    PreparedInstanceDeletePublication::Unwritten,
                ) => return Err(DeploymentError::IntentMismatch),
                _ => {}
            }
        }
        match history.original().publication() {
            PreparedInstanceDeletePublication::Unwritten
                if !self.publications.is_empty() || !self.worker_observations.is_empty() =>
            {
                return Err(DeploymentError::IntentMismatch);
            }
            PreparedInstanceDeletePublication::Committed(record) => {
                let observed = record.observation();
                self.publication(observed, false)?;
            }
            PreparedInstanceDeletePublication::Unwritten => {}
        }
        if !self.histories.contains(history) {
            self.histories.push(history.clone());
        }
        Ok(())
    }
    pub fn publication(
        &mut self,
        observed: &PreparedInstanceDeleteObservation,
        worker: bool,
    ) -> Result<(), DeploymentError> {
        if self
            .publications
            .iter()
            .chain(&self.worker_observations)
            .any(|old| {
                !same_delete(old.request(), observed.request())
                    || old.terminal_operation_id() != observed.terminal_operation_id()
                    || old.after_instance_version() != observed.after_instance_version()
            })
        {
            return Err(DeploymentError::IntentMismatch);
        }
        let records = if worker {
            &mut self.worker_observations
        } else {
            &mut self.publications
        };
        if !records.contains(observed) {
            records.push(observed.clone());
        }
        Ok(())
    }
}
pub(super) fn same_delete(
    a: &PreparedInstanceDeleteRequest,
    b: &PreparedInstanceDeleteRequest,
) -> bool {
    a.intent() == b.intent()
        && a.target() == b.target()
        && a.closure() == b.closure()
        && a.effect() == b.effect()
        && a.configuration() == b.configuration()
        && a.expected_instance_version() == b.expected_instance_version()
}
