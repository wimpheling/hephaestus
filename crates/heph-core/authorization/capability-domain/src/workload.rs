use runtime_types::RunId;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::errors::{CanonicalHasher, principal_canonicalize, snapshot_hash};
use crate::{
    AuthorityHash, AuthorizationSnapshotId, CapabilityBinding, CapabilityError,
    CapabilityOperation, CapabilityResource, GatewayInvocationId, RuntimeSessionId,
};

/// Durable workload kind authenticated by a runtime session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkloadKind {
    /// A durable agent instance.
    AgentInstance,
    /// A declared HTTP gateway.
    Gateway,
}

impl WorkloadKind {
    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Self::AgentInstance => "agent_instance",
            Self::Gateway => "gateway",
        }
    }
}

/// Exact immutable workload revision selected for dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WorkloadPrincipal {
    /// Workload category.
    pub kind: WorkloadKind,
    /// Durable workload identity.
    pub id: Uuid,
    /// Immutable selected revision identity.
    pub revision_id: Uuid,
}

impl WorkloadPrincipal {
    /// Creates an exact workload principal.
    #[must_use]
    pub const fn new(kind: WorkloadKind, id: Uuid, revision_id: Uuid) -> Self {
        Self {
            kind,
            id,
            revision_id,
        }
    }
}

/// Immutable dispatch-time maximum authority.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AuthorizationSnapshot {
    pub(super) id: AuthorizationSnapshotId,
    pub(super) principal: WorkloadPrincipal,
    pub(super) authorization_model_version: String,
    pub(super) bindings: Vec<CapabilityBinding>,
    pub(super) normalized_hash: AuthorityHash,
}

impl AuthorizationSnapshot {
    /// Creates a normalized immutable snapshot.
    ///
    /// # Errors
    ///
    /// Rejects an empty authorization model version, duplicate binding IDs,
    /// or more than one binding for the same symbolic slot.
    pub fn new(
        id: AuthorizationSnapshotId,
        principal: WorkloadPrincipal,
        authorization_model_version: impl Into<String>,
        mut bindings: Vec<CapabilityBinding>,
    ) -> Result<Self, CapabilityError> {
        let authorization_model_version = authorization_model_version.into();
        if authorization_model_version.is_empty() {
            return Err(CapabilityError::EmptyAuthorizationModelVersion);
        }
        bindings.sort_by(|left, right| left.slot.cmp(&right.slot));
        let mut binding_ids = BTreeSet::new();
        let mut slots = BTreeSet::new();
        for binding in &bindings {
            if !binding_ids.insert(binding.id) {
                return Err(CapabilityError::DuplicateBindingId(binding.id));
            }
            if !slots.insert(binding.slot.clone()) {
                return Err(CapabilityError::DuplicateBindingSlot(binding.slot.clone()));
            }
        }
        let normalized_hash = snapshot_hash(principal, &authorization_model_version, &bindings);
        Ok(Self {
            id,
            principal,
            authorization_model_version,
            bindings,
            normalized_hash,
        })
    }

    /// Returns the snapshot identifier.
    #[must_use]
    pub const fn id(&self) -> AuthorizationSnapshotId {
        self.id
    }

    /// Returns the exact workload revision whose bindings were snapshotted.
    #[must_use]
    pub const fn principal(&self) -> WorkloadPrincipal {
        self.principal
    }

    /// Returns the authorization model version used during resolution.
    #[must_use]
    pub fn authorization_model_version(&self) -> &str {
        &self.authorization_model_version
    }

    /// Returns normalized exact bindings.
    #[must_use]
    pub fn bindings(&self) -> impl ExactSizeIterator<Item = &CapabilityBinding> {
        self.bindings.iter()
    }

    /// Returns the immutable normalized snapshot hash.
    #[must_use]
    pub const fn normalized_hash(&self) -> AuthorityHash {
        self.normalized_hash
    }

    /// Returns whether the ceiling contains the exact operation and resource.
    #[must_use]
    pub fn allows(&self, resource: CapabilityResource, operation: CapabilityOperation) -> bool {
        self.bindings
            .iter()
            .any(|binding| binding.resource == resource && binding.grants(operation))
    }
}

/// One exact execution or gateway invocation authenticated by a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeInvocation {
    /// One durable agent run.
    Run(RunId),
    /// One bounded HTTP gateway invocation.
    Gateway(GatewayInvocationId),
}

impl RuntimeInvocation {
    fn canonicalize(self, hasher: &mut CanonicalHasher) {
        match self {
            Self::Run(id) => {
                hasher.text("run");
                hasher.uuid(id.as_uuid());
            }
            Self::Gateway(id) => {
                hasher.text("gateway");
                hasher.uuid(id.as_uuid());
            }
        }
    }
}

/// Immutable identity claims for one short-lived runtime session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RuntimeSessionIdentity {
    pub(super) id: RuntimeSessionId,
    pub(super) principal: WorkloadPrincipal,
    pub(super) invocation: RuntimeInvocation,
    pub(super) snapshot_id: AuthorizationSnapshotId,
    pub(super) snapshot_hash: AuthorityHash,
    pub(super) issued_at: OffsetDateTime,
    pub(super) expires_at: OffsetDateTime,
}

impl RuntimeSessionIdentity {
    /// Creates exact bounded identity claims for a runtime session.
    ///
    /// # Errors
    ///
    /// Rejects a non-positive validity interval or an invocation whose kind
    /// does not match the durable workload principal.
    pub fn new(
        id: RuntimeSessionId,
        principal: WorkloadPrincipal,
        invocation: RuntimeInvocation,
        snapshot: &AuthorizationSnapshot,
        issued_at: OffsetDateTime,
        expires_at: OffsetDateTime,
    ) -> Result<Self, CapabilityError> {
        if expires_at <= issued_at {
            return Err(CapabilityError::InvalidSessionValidity);
        }
        if principal != snapshot.principal {
            return Err(CapabilityError::SnapshotPrincipalMismatch);
        }
        let invocation_matches = matches!(
            (principal.kind, invocation),
            (WorkloadKind::AgentInstance, RuntimeInvocation::Run(_))
                | (WorkloadKind::Gateway, RuntimeInvocation::Gateway(_))
        );
        if !invocation_matches {
            return Err(CapabilityError::InvocationKindMismatch);
        }
        Ok(Self {
            id,
            principal,
            invocation,
            snapshot_id: snapshot.id,
            snapshot_hash: snapshot.normalized_hash,
            issued_at,
            expires_at,
        })
    }

    /// Returns the runtime session identifier.
    #[must_use]
    pub const fn id(&self) -> RuntimeSessionId {
        self.id
    }

    /// Returns the exact workload revision principal.
    #[must_use]
    pub const fn principal(&self) -> WorkloadPrincipal {
        self.principal
    }

    /// Returns the exact execution or HTTP invocation.
    #[must_use]
    pub const fn invocation(&self) -> RuntimeInvocation {
        self.invocation
    }

    /// Returns the immutable authority snapshot identifier.
    #[must_use]
    pub const fn snapshot_id(&self) -> AuthorizationSnapshotId {
        self.snapshot_id
    }

    /// Returns the immutable authority snapshot hash.
    #[must_use]
    pub const fn snapshot_hash(&self) -> AuthorityHash {
        self.snapshot_hash
    }

    /// Returns the issue time.
    #[must_use]
    pub const fn issued_at(&self) -> OffsetDateTime {
        self.issued_at
    }

    /// Returns the exclusive expiry time.
    #[must_use]
    pub const fn expires_at(&self) -> OffsetDateTime {
        self.expires_at
    }

    /// Returns whether these claims are valid at the given instant.
    #[must_use]
    pub fn is_valid_at(&self, now: OffsetDateTime) -> bool {
        now >= self.issued_at && now < self.expires_at
    }

    /// Returns a stable hash of these exact identity claims.
    #[must_use]
    pub fn normalized_hash(&self) -> AuthorityHash {
        let mut hasher = CanonicalHasher::new(b"hephaestus.runtime-session-identity.v1");
        hasher.uuid(self.id.as_uuid());
        principal_canonicalize(self.principal, &mut hasher);
        self.invocation.canonicalize(&mut hasher);
        hasher.uuid(self.snapshot_id.as_uuid());
        hasher.bytes(self.snapshot_hash.as_bytes());
        hasher.timestamp(self.issued_at);
        hasher.timestamp(self.expires_at);
        hasher.finish()
    }
}
