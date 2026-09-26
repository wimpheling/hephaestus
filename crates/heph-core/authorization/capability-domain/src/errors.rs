use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::credentials::AuthorityHash;
use crate::{
    CapabilityBinding, CapabilityBindingId, CapabilityOperation, CapabilityResourceKind,
    CapabilitySlotKey, MAX_OPERATIONS_PER_CAPABILITY, WorkloadPrincipal,
};

/// Invalid capability or runtime authority value.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum CapabilityError {
    /// A symbolic slot key was malformed or outside its bound.
    #[error(
        "capability slot key must start with a lowercase letter and contain 1 to 64 lowercase ASCII letters, digits, underscores, or hyphens"
    )]
    InvalidSlotKey,
    /// The input operation collection contained the same operation twice.
    #[error("duplicate capability operation {0:?}")]
    DuplicateOperation(CapabilityOperation),
    /// The operation set was empty.
    #[error("a capability must declare or grant at least one operation")]
    EmptyOperationSet,
    /// The operation set exceeded its maximum size.
    #[error("a capability may contain at most {MAX_OPERATIONS_PER_CAPABILITY} operations")]
    TooManyOperations,
    /// The operation is undefined for the selected resource category.
    #[error("operation {operation:?} is not legal for resource kind {resource_kind:?}")]
    IllegalOperation {
        /// Invalid operation.
        operation: CapabilityOperation,
        /// Selected resource category.
        resource_kind: CapabilityResourceKind,
    },
    /// One operation was classified as both required and optional.
    #[error("operation {0:?} cannot be both required and optional")]
    OperationRequiredAndOptional(CapabilityOperation),
    /// The selected resource category did not match the declared category.
    #[error("resource kind {actual:?} is incompatible with required kind {expected:?}")]
    IncompatibleResourceKind {
        /// Declared resource category.
        expected: CapabilityResourceKind,
        /// Selected resource category.
        actual: CapabilityResourceKind,
    },
    /// An explicit grant omitted a required operation.
    #[error("binding is missing required operation {0:?}")]
    MissingRequiredOperation(CapabilityOperation),
    /// An explicit grant exceeded the release declaration.
    #[error("binding grants undeclared operation {0:?}")]
    UndeclaredOperation(CapabilityOperation),
    /// A snapshot contained one binding identifier more than once.
    #[error("duplicate capability binding ID {0}")]
    DuplicateBindingId(CapabilityBindingId),
    /// A snapshot contained more than one binding for a symbolic slot.
    #[error("duplicate capability binding slot {0}")]
    DuplicateBindingSlot(CapabilitySlotKey),
    /// The authorization model version was empty.
    #[error("authorization model version cannot be empty")]
    EmptyAuthorizationModelVersion,
    /// The runtime session expiry did not follow its issue time.
    #[error("runtime session expiry must be later than its issue time")]
    InvalidSessionValidity,
    /// A runtime credential issuance generation was zero.
    #[error("runtime credential issuance generation must be positive")]
    InvalidCredentialGeneration,
    /// A runtime invocation kind did not match its workload principal kind.
    #[error("runtime invocation kind does not match workload principal kind")]
    InvocationKindMismatch,
    /// The snapshot belongs to a different workload revision.
    #[error("authorization snapshot belongs to a different workload principal")]
    SnapshotPrincipalMismatch,
    /// Runtime identity claims did not reference the supplied exact snapshot.
    #[error("runtime identity does not reference the supplied exact authorization snapshot")]
    SnapshotIdentityMismatch,
}

pub fn operation_set(
    operations: impl IntoIterator<Item = CapabilityOperation>,
) -> Result<BTreeSet<CapabilityOperation>, CapabilityError> {
    let mut normalized = BTreeSet::new();
    for operation in operations {
        if !normalized.insert(operation) {
            return Err(CapabilityError::DuplicateOperation(operation));
        }
        if normalized.len() > MAX_OPERATIONS_PER_CAPABILITY {
            return Err(CapabilityError::TooManyOperations);
        }
    }
    Ok(normalized)
}

pub fn validate_operations(
    resource_kind: CapabilityResourceKind,
    operations: &BTreeSet<CapabilityOperation>,
) -> Result<(), CapabilityError> {
    if let Some(operation) = operations
        .iter()
        .find(|operation| !operation.is_legal_for(resource_kind))
        .copied()
    {
        return Err(CapabilityError::IllegalOperation {
            operation,
            resource_kind,
        });
    }
    Ok(())
}

pub fn snapshot_hash(
    principal: WorkloadPrincipal,
    authorization_model_version: &str,
    bindings: &[CapabilityBinding],
) -> AuthorityHash {
    let mut hasher = CanonicalHasher::new(b"hephaestus.authorization-snapshot.v1");
    principal_canonicalize(principal, &mut hasher);
    hasher.text(authorization_model_version);
    hasher.usize(bindings.len());
    for binding in bindings {
        hasher.bytes(binding.normalized_hash().as_bytes());
    }
    hasher.finish()
}

pub fn principal_canonicalize(principal: WorkloadPrincipal, hasher: &mut CanonicalHasher) {
    hasher.text(principal.kind.as_str());
    hasher.uuid(principal.id);
    hasher.uuid(principal.revision_id);
}

pub struct CanonicalHasher(Sha256);

impl CanonicalHasher {
    pub fn new(domain: &[u8]) -> Self {
        let mut hasher = Self(Sha256::new());
        hasher.bytes(domain);
        hasher
    }

    pub fn bytes(&mut self, value: &[u8]) {
        self.0.update(value.len().to_be_bytes());
        self.0.update(value);
    }

    pub fn text(&mut self, value: &str) {
        self.bytes(value.as_bytes());
    }

    pub fn uuid(&mut self, value: Uuid) {
        self.bytes(value.as_bytes());
    }

    pub fn boolean(&mut self, value: bool) {
        self.bytes(&[u8::from(value)]);
    }

    pub fn usize(&mut self, value: usize) {
        self.bytes(&value.to_be_bytes());
    }

    pub fn operations(&mut self, operations: &BTreeSet<CapabilityOperation>) {
        let mut names = operations
            .iter()
            .map(|operation| operation.as_str())
            .collect::<Vec<_>>();
        names.sort_unstable();
        self.usize(names.len());
        for name in names {
            self.text(name);
        }
    }

    pub fn timestamp(&mut self, value: OffsetDateTime) {
        self.bytes(&value.unix_timestamp_nanos().to_be_bytes());
    }

    pub fn finish(self) -> AuthorityHash {
        AuthorityHash(self.0.finalize().into())
    }
}
