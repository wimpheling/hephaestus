//! Fresh removal admission and immutable original continuation data.
mod terminal;
use crate::{
    DeploymentAdmission, DeploymentError, DeploymentExecutionProfile, DeploymentOperation,
    EffectClaim, PlannedResourceIdentity, PlatformPolicyObservation, ResourceAction,
    ResourceOwnership,
};

/// Trusted composition comparison data; grants no provider ownership or authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoveResumeConfiguration {
    namespace: String,
    host: String,
    platform: PlatformPolicyObservation,
    profile: DeploymentExecutionProfile,
}
impl RemoveResumeConfiguration {
    /// Checks actual provider labels and immutable execution configuration.
    ///
    /// # Errors
    /// Rejects labels outside the existing cleanup comparison bounds.
    pub fn new(
        namespace: String,
        host: String,
        platform: PlatformPolicyObservation,
        profile: DeploymentExecutionProfile,
    ) -> Result<Self, DeploymentError> {
        let valid = |value: &str| {
            !value.is_empty()
                && value.len() <= 128
                && !matches!(value, "." | "..")
                && value.bytes().all(|byte| {
                    byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-')
                })
        };
        if !valid(&namespace) || !valid(&host) {
            return Err(DeploymentError::IntentMismatch);
        }
        Ok(Self {
            namespace,
            host,
            platform,
            profile,
        })
    }
    /// Returns actual provider namespace comparison data, never disk ownership.
    #[must_use]
    pub fn namespace(&self) -> &str {
        &self.namespace
    }
    /// Returns actual provider host comparison data.
    #[must_use]
    pub fn host(&self) -> &str {
        &self.host
    }
    /// Returns immutable configured platform observations.
    #[must_use]
    pub const fn platform(&self) -> &PlatformPolicyObservation {
        &self.platform
    }
    /// Returns the explicitly configured preparation semantics.
    #[must_use]
    pub const fn profile(&self) -> DeploymentExecutionProfile {
        self.profile
    }
}

pub use crate::instance_closure_publication::OriginalInstanceClosure;

/// One immutable original removal claim, with existing closure when relevant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoveContinuation {
    /// Original claim whose actor and operation must never be replaced.
    pub claim: EffectClaim,
    /// Exact original instance Drain closure; no physical cleanup is asserted.
    pub closure: Option<OriginalInstanceClosure>,
}
/// Fresh current manager admission plus independently compared old work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoveResumeAdmission {
    admission: DeploymentAdmission,
    continuations: Vec<RemoveContinuation>,
    closures: Vec<OriginalInstanceClosure>,
    pending: Vec<crate::PendingInstanceDrainClosure>,
    detach: Vec<crate::PreparedInstanceDetachRecovery>,
    terminal: Vec<crate::PreparedInstanceDeleteHistory>,
}
impl RemoveResumeAdmission {
    /// Checks active continuation shape against the exact immutable admitted deployment.
    ///
    /// This compatibility constructor retains supplied active closures; production
    /// adapters use `with_complete_closures` to preserve inactive history too.
    ///
    /// Construction conveys no database admission or provider proof.
    ///
    /// # Errors
    /// Rejects unsupported actions, duplicate targets and substituted old pins.
    pub fn new(
        admission: DeploymentAdmission,
        continuations: Vec<RemoveContinuation>,
    ) -> Result<Self, DeploymentError> {
        let mut names = std::collections::BTreeSet::new();
        if admission.receipt.command.operation() != DeploymentOperation::Remove {
            return Err(DeploymentError::IntentMismatch);
        }
        for item in &continuations {
            let claim = &item.claim;
            let plan = admission
                .snapshot
                .intent
                .resources()
                .get(&claim.resource)
                .ok_or(DeploymentError::IntentMismatch)?;
            if !names.insert(claim.resource.clone())
                || claim.deployment_id != admission.snapshot.intent.id()
                || claim.command == admission.receipt.command
                || claim.command.operation() != DeploymentOperation::Remove
                || claim.provenance.actor_id != claim.command.actor_id()
                || claim.generation == 0
                || claim.resource_version == 0
                || claim.provenance.request_id.as_uuid().is_nil()
                || plan.ownership() != ResourceOwnership::Owned
                || plan.identity() != claim.identity
                || plan.input_hash() != claim.input_hash
                || !matches!(
                    claim.action,
                    ResourceAction::Drain | ResourceAction::Detach | ResourceAction::Retain
                )
            {
                return Err(DeploymentError::IntentMismatch);
            }
            if let Some(closure) = &item.closure {
                closure.validate(claim)?;
            } else if matches!(claim.identity, PlannedResourceIdentity::Instance { .. }) {
                return Err(DeploymentError::ReconciliationRequired);
            }
        }
        let closures = continuations
            .iter()
            .filter_map(|item| item.closure.clone())
            .collect();
        Ok(Self {
            admission,
            continuations,
            closures,
            pending: Vec::new(),
            detach: Vec::new(),
            terminal: Vec::new(),
        })
    }
    /// Preserves the complete native original closure inventory separately from active work.
    ///
    /// Inactive closures remain comparison data, never completed-effect or physical
    /// cleanup proof. Adapters must independently qualify the complete persisted set.
    ///
    /// # Errors
    /// Rejects duplicate, foreign or substituted originals and missing active closures.
    pub fn with_complete_closures(
        admission: DeploymentAdmission,
        continuations: Vec<RemoveContinuation>,
        closures: Vec<OriginalInstanceClosure>,
    ) -> Result<Self, DeploymentError> {
        let mut result = Self::new(admission, continuations)?;
        let mut names = std::collections::BTreeMap::new();
        let mut attempts = std::collections::BTreeSet::new();
        let mut removals = std::collections::BTreeSet::new();
        for closure in &closures {
            let claim = &closure.drain;
            let plan = result
                .admission
                .snapshot
                .intent
                .resources()
                .get(&claim.resource)
                .ok_or(DeploymentError::IntentMismatch)?;
            closure.validate(claim)?;
            if claim.deployment_id != result.admission.snapshot.intent.id()
                || claim.command == result.admission.receipt.command
                || plan.ownership() != ResourceOwnership::Owned
                || plan.identity() != claim.identity
                || plan.input_hash() != claim.input_hash
                || names.insert(claim.resource.clone(), closure).is_some()
                || !attempts.insert(claim.provenance.attempt_id)
                || !removals.insert(closure.removal_id.as_uuid())
            {
                return Err(DeploymentError::IntentMismatch);
            }
        }
        for item in &result.continuations {
            if let Some(closure) = &item.closure {
                if names.get(&closure.drain.resource).copied() != Some(closure) {
                    return Err(DeploymentError::ReconciliationRequired);
                }
            }
        }
        result.closures = closures;
        Ok(result)
    }
    /// Retains explicitly qualified pending projections alongside complete closed history.
    ///
    /// Only an exact supplied pending match permits an Instance continuation without
    /// a closure. This comparison cannot establish missing native rows or authority.
    /// The existing strict constructors retain their original denial behavior.
    ///
    /// # Errors
    /// Rejects missing, orphan, duplicate or cross-inventory conflicting originals,
    /// changed snapshots/current commands and substituted pending continuation claims.
    pub fn with_pending_instance_drains(
        admission: DeploymentAdmission,
        continuations: Vec<RemoveContinuation>,
        closures: Vec<OriginalInstanceClosure>,
        pending: Vec<crate::PendingInstanceDrainClosure>,
    ) -> Result<Self, DeploymentError> {
        let mut names = std::collections::BTreeMap::new();
        for item in &pending {
            if item.snapshot() != &admission.snapshot
                || item.current_command() != admission.receipt.command
                || admission.receipt.deployment_id != admission.snapshot.intent.id()
                || names.insert(item.drain().resource.clone(), item).is_some()
            {
                return Err(DeploymentError::IntentMismatch);
            }
        }
        let closed = compared_continuations(&continuations, &names)?;
        compare_inventory_attempts(&continuations, &closures, &names)?;
        let mut result = Self::with_complete_closures(admission, closed, closures)?;
        result.continuations = continuations;
        result.pending = pending;
        Ok(result)
    }
    /// Returns explicit pending comparison projections, never absence or admission proof.
    #[must_use]
    pub fn pending_instance_drains(&self) -> &[crate::PendingInstanceDrainClosure] {
        &self.pending
    }
    /// Returns all qualified original closures, including inactive completed history.
    ///
    /// Presence is immutable comparison data, not an outcome or IO authorization.
    #[must_use]
    pub fn closures(&self) -> &[OriginalInstanceClosure] {
        &self.closures
    }
    /// Returns the new current manager's real command admission.
    #[must_use]
    pub const fn admission(&self) -> &DeploymentAdmission {
        &self.admission
    }
    /// Returns original claims and closures as comparison data only.
    #[must_use]
    pub fn continuations(&self) -> &[RemoveContinuation] {
        &self.continuations
    }
}

fn compared_continuations(
    continuations: &[RemoveContinuation],
    pending: &std::collections::BTreeMap<
        capability_domain::CapabilitySlotKey,
        &crate::PendingInstanceDrainClosure,
    >,
) -> Result<Vec<RemoveContinuation>, DeploymentError> {
    let mut names = std::collections::BTreeSet::new();
    let mut matched = std::collections::BTreeSet::new();
    let mut closed = Vec::new();
    for item in continuations {
        if !names.insert(item.claim.resource.clone()) {
            return Err(DeploymentError::IntentMismatch);
        }
        if let Some(original) = pending.get(&item.claim.resource) {
            if item.closure.is_some() || original.drain() != &item.claim {
                return Err(DeploymentError::IntentMismatch);
            }
            matched.insert(item.claim.resource.clone());
        } else {
            closed.push(item.clone());
        }
    }
    if matched.len() != pending.len() {
        return Err(DeploymentError::IntentMismatch);
    }
    Ok(closed)
}

fn compare_inventory_attempts(
    continuations: &[RemoveContinuation],
    closures: &[OriginalInstanceClosure],
    pending: &std::collections::BTreeMap<
        capability_domain::CapabilitySlotKey,
        &crate::PendingInstanceDrainClosure,
    >,
) -> Result<(), DeploymentError> {
    let mut attempts = std::collections::BTreeMap::new();
    for item in continuations {
        if attempts
            .insert(item.claim.provenance.attempt_id, &item.claim.resource)
            .is_some()
        {
            return Err(DeploymentError::IntentMismatch);
        }
    }
    for closure in closures {
        let claim = &closure.drain;
        if pending.contains_key(&claim.resource) {
            return Err(DeploymentError::IntentMismatch);
        }
        if attempts
            .insert(claim.provenance.attempt_id, &claim.resource)
            .is_some_and(|resource| resource != &claim.resource)
        {
            return Err(DeploymentError::IntentMismatch);
        }
    }
    Ok(())
}
