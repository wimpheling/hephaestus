use capability_domain::CapabilitySlotKey;
use event_application::CommittedMutation;
use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
use release_domain::ContentHash;
use serde::Serialize;
use uuid::Uuid;

use crate::{
    DeploymentAttemptId, DeploymentCommandId, DeploymentError, DeploymentId, DeploymentIntent,
    DeploymentLifecycle, DeploymentOperation, DiagnosticCode, PlannedResourceIdentity,
    ResourceAction,
};

/// Actor and operation scoped logical identity; request provenance is separate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct CommandIdentity {
    id: DeploymentCommandId,
    actor_id: UserId,
    idempotency_id: RequestId,
    operation: DeploymentOperation,
}

impl CommandIdentity {
    /// Derives command identity from an authenticated actor and stable logical seed.
    ///
    /// Callers must replace default request-based idempotency IDs for retryable work.
    ///
    /// # Errors
    /// Rejects nil actor or idempotency identities.
    pub fn from_identity(
        identity: &AuthenticatedIdentity,
        operation: DeploymentOperation,
    ) -> Result<Self, DeploymentError> {
        Self::from_recorded(identity.user_id, identity.idempotency_id, operation)
    }

    /// Reconstructs actor-scoped data identity from immutable recorded fields.
    ///
    /// This pure derivation supplies no authenticated identity or authorization.
    /// Effects must validate a real fresh middleware identity and live authority.
    ///
    /// # Errors
    /// Rejects nil recorded actor or logical idempotency identity.
    pub fn from_recorded(
        actor_id: UserId,
        idempotency_id: RequestId,
        operation: DeploymentOperation,
    ) -> Result<Self, DeploymentError> {
        if actor_id.as_uuid().is_nil() || idempotency_id.as_uuid().is_nil() {
            return Err(DeploymentError::InvalidIdentifier);
        }
        let mut bytes = b"hephaestus-recipe-command-v1\0".to_vec();
        bytes.extend_from_slice(actor_id.as_uuid().as_bytes());
        bytes.extend_from_slice(operation.as_str().as_bytes());
        Ok(Self {
            id: DeploymentCommandId::from_uuid(Uuid::new_v5(&idempotency_id.as_uuid(), &bytes))?,
            actor_id,
            idempotency_id,
            operation,
        })
    }
    /// Returns the stable logical command ID.
    #[must_use]
    pub const fn id(self) -> DeploymentCommandId {
        self.id
    }
    /// Returns the actor bound to this command.
    #[must_use]
    pub const fn actor_id(self) -> UserId {
        self.actor_id
    }
    /// Returns stable occurrence identity for committed events and receipts.
    #[must_use]
    pub const fn idempotency_id(self) -> RequestId {
        self.idempotency_id
    }
    /// Returns the command's exact operation.
    #[must_use]
    pub const fn operation(self) -> DeploymentOperation {
        self.operation
    }
    /// Checks exact actor, logical identity, and operation for an incoming attempt.
    ///
    /// # Errors
    /// Rejects a command borrowed from a different actor, operation, or logical seed.
    pub fn validate(
        self,
        identity: &AuthenticatedIdentity,
        operation: DeploymentOperation,
    ) -> Result<(), DeploymentError> {
        if self == Self::from_identity(identity, operation)? {
            Ok(())
        } else {
            Err(DeploymentError::InputConflict)
        }
    }
}

/// Immutable install admission input; constructor checks do not authorize installation.
#[derive(Debug, Clone)]
pub struct InstallDeployment {
    /// Stable authenticated command identity.
    pub command: CommandIdentity,
    /// Exact validated deployment declaration and resolved graph.
    pub intent: DeploymentIntent,
}

/// Policy-bounded removal admission with optimistic concurrency.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RemoveDeployment {
    /// Stable authenticated removal identity.
    pub command: CommandIdentity,
    /// Exact deployment whose immutable policies govern removal.
    pub deployment_id: DeploymentId,
    /// Expected current deployment version, included in command fingerprint.
    pub expected_version: u64,
}

impl RemoveDeployment {
    /// Returns exact logical removal input identity, independent of request attempts.
    ///
    /// # Errors
    /// Returns an error if canonical serialization unexpectedly fails.
    pub fn input_hash(self) -> Result<ContentHash, DeploymentError> {
        serde_json::to_vec(&(
            "hephaestus-recipe-remove-v1",
            self.deployment_id,
            self.expected_version,
        ))
        .map(|bytes| ContentHash::digest(&bytes))
        .map_err(|_| DeploymentError::Serialization)
    }
}

/// Per-attempt provenance, separate from actor-scoped logical command identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct AttemptProvenance {
    /// Unique attempt identity, never reused to take another provider action.
    pub attempt_id: DeploymentAttemptId,
    /// Actor whose current authority is checked for this attempt.
    pub actor_id: UserId,
    /// Per-attempt request correlation identity.
    pub request_id: RequestId,
}

impl AttemptProvenance {
    /// Records request provenance for a fresh effect or reconciliation attempt.
    ///
    /// # Errors
    /// Rejects nil actor or request identities.
    pub const fn new(
        identity: &AuthenticatedIdentity,
        attempt_id: DeploymentAttemptId,
    ) -> Result<Self, DeploymentError> {
        if identity.user_id.as_uuid().is_nil() || identity.request_id.as_uuid().is_nil() {
            return Err(DeploymentError::InvalidIdentifier);
        }
        Ok(Self {
            attempt_id,
            actor_id: identity.user_id,
            request_id: identity.request_id,
        })
    }
}

/// Compare-and-swap request to durably claim a bounded resource action.
#[derive(Debug, Clone)]
pub struct ResourceEffect {
    /// Previously admitted logical command.
    pub command: CommandIdentity,
    /// Exact deployment owning this progress record.
    pub deployment_id: DeploymentId,
    /// Named resource from immutable intent.
    pub resource: CapabilitySlotKey,
    /// Expected deployment version.
    pub expected_deployment_version: u64,
    /// Expected resource progress version.
    pub expected_resource_version: u64,
    /// Closed requested effect.
    pub action: ResourceAction,
    /// Fresh per-attempt provenance.
    pub provenance: AttemptProvenance,
}

/// Committed claim returned before provider execution; never a reusable grant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectClaim {
    /// Exact logical command identity.
    pub command: CommandIdentity,
    /// Deployment containing immutable intent and progress.
    pub deployment_id: DeploymentId,
    /// Exact named resource.
    pub resource: CapabilitySlotKey,
    /// Predicted or externally bound identity to reconcile.
    pub identity: PlannedResourceIdentity,
    /// Exact immutable desired-resource fingerprint.
    pub input_hash: ContentHash,
    /// Committed resource progress version required by completion CAS.
    pub resource_version: u64,
    /// Monotonic attempt fencing generation; expiry alone never permits reuse.
    pub generation: u64,
    /// Exact permitted action for this claim.
    pub action: ResourceAction,
    /// Actor/request/attempt provenance persisted with claim.
    pub provenance: AttemptProvenance,
}

/// Definite effect outcome after the exact provider identity was checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectOutcome {
    /// Provider confirmed the requested action and its required safety conditions.
    Applied,
    /// Provider proved the effect did not succeed.
    Failed(DiagnosticCode),
}

/// Provider-independent confirmed evidence; adapters retain concrete handles privately.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EffectEvidence {
    /// Exact observed stable identity, which must match the durable claim.
    pub identity: PlannedResourceIdentity,
    /// Exact verified configuration fingerprint.
    pub input_hash: ContentHash,
    /// Definite result; ambiguous results use a separate repository method.
    pub outcome: EffectOutcome,
}

/// Reconciliation result without assuming an expired claim implies absence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReconciledOutcome {
    /// Exact provider identity and configuration prove the effect succeeded.
    Applied(EffectEvidence),
    /// Provider proves the effect did not occur; a fresh claim may be considered.
    NotApplied,
    /// Outcome or detach/fencing safety remains unproven; retain recovery state.
    Unproven(DiagnosticCode),
}

/// Durable result associated with unchanged logical command input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandReceipt {
    /// Original actor and operation scoped command identity.
    pub command: CommandIdentity,
    /// Exact deployment committed by the operation.
    pub deployment_id: DeploymentId,
    /// Original immutable command input fingerprint.
    pub input_hash: ContentHash,
    /// Lifecycle recorded by this receipt.
    pub lifecycle: DeploymentLifecycle,
    /// Committed deployment version.
    pub deployment_version: u64,
    /// Canonical committed project event and cursor, never an uncommitted prediction.
    pub event: CommittedMutation,
}
