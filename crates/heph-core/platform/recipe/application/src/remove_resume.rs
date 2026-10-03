//! Fresh removal admission and immutable original continuation data.
use crate::{
    DeploymentAdmission, DeploymentError, DeploymentExecutionProfile, DeploymentOperation,
    EffectClaim, PlannedResourceIdentity, PlatformPolicyObservation, ResourceAction,
    ResourceOwnership,
};
use identity_domain::{RequestId, UserId};
use release_domain::{ContentHash, InstanceRemovalId, ReleaseCommandKey};
use uuid::Uuid;

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

/// Immutable original permanent closure, never the old actor's credentials.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OriginalInstanceClosure {
    /// Exact original Drain claim that created the permanent request.
    pub drain: EffectClaim,
    /// Actual request derived from the original Drain attempt.
    pub removal_id: InstanceRemovalId,
    /// Original owning release command key.
    pub command_key: ReleaseCommandKey,
    /// Instance version before permanent closure.
    pub expected_version: u64,
    /// Original recorded creator, comparison data only.
    pub actor: UserId,
    /// Original recorded request, comparison data only.
    pub request: RequestId,
    /// Original exact request fingerprint.
    pub input_hash: ContentHash,
}
impl OriginalInstanceClosure {
    /// Compares closure identities to the original actual Drain claim.
    ///
    /// # Errors
    /// Rejects substituted claims, identities, creator or version.
    pub fn validate(&self, claim: &EffectClaim) -> Result<(), DeploymentError> {
        let drain = &self.drain;
        if drain.deployment_id != claim.deployment_id
            || drain.resource != claim.resource
            || drain.identity != claim.identity
            || drain.input_hash != claim.input_hash
        {
            return Err(DeploymentError::IntentMismatch);
        }
        let PlannedResourceIdentity::Instance { id, .. } = drain.identity else {
            return Err(DeploymentError::IntentMismatch);
        };
        let mut bytes = b"hephaestus-prepared-instance-removal-v1\0".to_vec();
        bytes.extend_from_slice(id.as_uuid().as_bytes());
        let expected = Uuid::new_v5(&drain.provenance.attempt_id.as_uuid(), &bytes);
        let key = ReleaseCommandKey::derive(
            "prepared_instance_remove_v1",
            &[
                id.as_uuid().as_bytes(),
                drain.provenance.attempt_id.as_uuid().as_bytes(),
            ],
        );
        if drain.provenance.actor_id != drain.command.actor_id()
            || drain.provenance.request_id.as_uuid().is_nil()
            || drain.generation == 0
            || drain.resource_version == 0
            || drain.action != ResourceAction::Drain
            || drain.command.operation() != DeploymentOperation::Remove
            || self.removal_id.as_uuid() != expected
            || self.command_key != key
            || self.actor != drain.provenance.actor_id
            || self.request.as_uuid().is_nil()
            || self.expected_version == 0
            || self.expected_version > i64::MAX.cast_unsigned()
        {
            return Err(DeploymentError::IntentMismatch);
        }
        Ok(())
    }
}
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
}
impl RemoveResumeAdmission {
    /// Checks shape against the exact immutable admitted deployment.
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
        Ok(Self {
            admission,
            continuations,
        })
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
