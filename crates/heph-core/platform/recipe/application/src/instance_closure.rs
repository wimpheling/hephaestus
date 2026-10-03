//! Qualified permanent closure admission, separate from runtime cleanup evidence.
use crate::{
    DeploymentError, DeploymentIntent, DeploymentOperation, EffectClaim,
    PreparedInstanceSafetyTarget, ResourceAction,
};
use async_trait::async_trait;
use identity_domain::AuthenticatedIdentity;
use release_domain::{InstanceRemovalId, ReleaseCommandKey};
use uuid::Uuid;

/// Exact current Drain claim and original import; no IO or completion authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedInstanceClosureRequest {
    intent: DeploymentIntent,
    target: PreparedInstanceSafetyTarget,
    drain: EffectClaim,
    expected_instance_version: u64,
    removal: InstanceRemovalId,
    key: ReleaseCommandKey,
}
impl PreparedInstanceClosureRequest {
    /// Derives stable closure identities from an actual current Remove Drain claim.
    ///
    /// Shape checking does not establish admission, scope ownership or cleanup.
    ///
    /// # Errors
    /// Rejects changed original pins, non-Drain claims or invalid version/provenance.
    pub fn new(
        intent: DeploymentIntent,
        target: PreparedInstanceSafetyTarget,
        drain: EffectClaim,
        expected_instance_version: u64,
    ) -> Result<Self, DeploymentError> {
        let original = target.original();
        let planned = intent
            .resources()
            .get(&original.resource)
            .ok_or(DeploymentError::IntentMismatch)?;
        let same_deployment = drain.deployment_id == original.deployment_id;
        let same_resource = drain.resource == original.resource;
        let same_identity = drain.identity == original.identity;
        if intent.id() != original.deployment_id
            || intent.project_id() != target.prepared().project_id()
            || planned.identity() != original.identity
            || planned.input_hash() != original.input_hash
            || !same_deployment
            || !same_resource
            || !same_identity
            || drain.input_hash != original.input_hash
            || drain.action != ResourceAction::Drain
            || drain.command.operation() != DeploymentOperation::Remove
            || drain.provenance.actor_id != drain.command.actor_id()
            || drain.resource_version == 0
            || drain.generation == 0
            || drain.provenance.request_id.as_uuid().is_nil()
            || expected_instance_version == 0
            || expected_instance_version > i64::MAX.cast_unsigned()
        {
            return Err(DeploymentError::IntentMismatch);
        }
        let instance = target.prepared().instance_id().as_uuid();
        let attempt = drain.provenance.attempt_id.as_uuid();
        let mut bytes = b"hephaestus-prepared-instance-removal-v1\0".to_vec();
        bytes.extend_from_slice(instance.as_bytes());
        let removal = InstanceRemovalId::from_uuid(Uuid::new_v5(&attempt, &bytes));
        let key = ReleaseCommandKey::derive(
            "prepared_instance_remove_v1",
            &[instance.as_bytes(), attempt.as_bytes()],
        );
        Ok(Self {
            intent,
            target,
            drain,
            expected_instance_version,
            removal,
            key,
        })
    }
    /// Returns checked immutable intent, independently compared to authoritative storage.
    #[must_use]
    pub const fn intent(&self) -> &DeploymentIntent {
        &self.intent
    }
    /// Returns complete original comparison data, never installer credentials.
    #[must_use]
    pub const fn target(&self) -> &PreparedInstanceSafetyTarget {
        &self.target
    }
    /// Returns the exact current admitted Drain claim to recheck under row locks.
    #[must_use]
    pub const fn drain(&self) -> &EffectClaim {
        &self.drain
    }
    /// Returns the version expected before the first permanent closure.
    #[must_use]
    pub const fn expected_instance_version(&self) -> u64 {
        self.expected_instance_version
    }
    /// Returns the stable immutable removal identity for this original Drain claim.
    #[must_use]
    pub const fn removal_id(&self) -> InstanceRemovalId {
        self.removal
    }
    /// Returns the stable owning release-service command identity.
    #[must_use]
    pub const fn command_key(&self) -> ReleaseCommandKey {
        self.key
    }
}
/// Committed scheduling closure; cancellation delivery is not a cleanup receipt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedInstanceClosureAdmission {
    /// Exact original target compared under current manager authority.
    pub target: PreparedInstanceSafetyTarget,
    /// Immutable actual removal request, never inferred from a false gate.
    pub removal_id: InstanceRemovalId,
    /// Whether the exact request already committed; no gate/version is reopened.
    pub replayed: bool,
}
/// Permanently closes one prepared instance under current ALL-owned cleanup rights.
///
/// The implementation must reserve the complete original preparation before any
/// parent row lock, then use late nonblocking participant locks. No provider or
/// transport IO occurs in its transaction. Runtime cleanup remains independently
/// qualified by the read-only safety observer.
#[async_trait]
pub trait PreparedInstanceClosureRepository: Send + Sync + 'static {
    /// Admits exact permanent closure and durable cancellation in one transaction.
    ///
    /// # Errors
    /// Denies unsupported adapters, roles, changed claims/configuration, stale CAS,
    /// missing original creation, permission loss or participant contention.
    async fn close_for_remove(
        &self,
        _identity: &AuthenticatedIdentity,
        _request: &PreparedInstanceClosureRequest,
    ) -> Result<PreparedInstanceClosureAdmission, DeploymentError> {
        Err(DeploymentError::InvalidAction)
    }
}
