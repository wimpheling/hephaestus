use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use uuid::Uuid;

use crate::errors::{CanonicalHasher, operation_set, validate_operations};
use crate::{
    AuthorityHash, CapabilityBindingId, CapabilityError, CapabilityOperation,
    CapabilityRequirementId, CapabilityResourceKind, CapabilitySlotKey,
};

/// A symbolic release request for one kind of controlled resource.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "CapabilityRequirementWire")]
pub struct CapabilityRequirement {
    pub(crate) id: CapabilityRequirementId,
    pub(crate) slot: CapabilitySlotKey,
    resource_kind: CapabilityResourceKind,
    required_operations: BTreeSet<CapabilityOperation>,
    optional_operations: BTreeSet<CapabilityOperation>,
    slot_required: bool,
}

impl CapabilityRequirement {
    /// Validates and constructs one normalized requirement.
    ///
    /// # Errors
    ///
    /// Rejects empty declarations, oversized declarations, illegal resource
    /// operations, duplicate input operations, and operations present in both
    /// the required and optional sets.
    pub fn new(
        id: CapabilityRequirementId,
        slot: CapabilitySlotKey,
        resource_kind: CapabilityResourceKind,
        required_operations: impl IntoIterator<Item = CapabilityOperation>,
        optional_operations: impl IntoIterator<Item = CapabilityOperation>,
        slot_required: bool,
    ) -> Result<Self, CapabilityError> {
        let required_operations = operation_set(required_operations)?;
        let optional_operations = operation_set(optional_operations)?;
        if required_operations.is_empty() && optional_operations.is_empty() {
            return Err(CapabilityError::EmptyOperationSet);
        }
        validate_operations(resource_kind, &required_operations)?;
        validate_operations(resource_kind, &optional_operations)?;
        if let Some(operation) = required_operations
            .intersection(&optional_operations)
            .next()
            .copied()
        {
            return Err(CapabilityError::OperationRequiredAndOptional(operation));
        }
        Ok(Self {
            id,
            slot,
            resource_kind,
            required_operations,
            optional_operations,
            slot_required,
        })
    }

    /// Returns the stable requirement identifier.
    #[must_use]
    pub const fn id(&self) -> CapabilityRequirementId {
        self.id
    }

    /// Returns the symbolic release slot.
    #[must_use]
    pub const fn slot(&self) -> &CapabilitySlotKey {
        &self.slot
    }

    /// Returns the required resource category.
    #[must_use]
    pub const fn resource_kind(&self) -> CapabilityResourceKind {
        self.resource_kind
    }

    /// Returns required operations in normalized order.
    #[must_use]
    pub fn required_operations(&self) -> impl ExactSizeIterator<Item = CapabilityOperation> + '_ {
        self.required_operations.iter().copied()
    }

    /// Returns optional operations in normalized order.
    #[must_use]
    pub fn optional_operations(&self) -> impl ExactSizeIterator<Item = CapabilityOperation> + '_ {
        self.optional_operations.iter().copied()
    }

    /// Returns whether the slot itself must be bound before dispatch.
    #[must_use]
    pub const fn slot_required(&self) -> bool {
        self.slot_required
    }

    /// Returns whether the operation is inside the declared ceiling.
    #[must_use]
    pub fn declares(&self, operation: CapabilityOperation) -> bool {
        self.required_operations.contains(&operation)
            || self.optional_operations.contains(&operation)
    }

    /// Returns a stable hash of the normalized requirement.
    #[must_use]
    pub fn normalized_hash(&self) -> AuthorityHash {
        let mut hasher = CanonicalHasher::new(b"hephaestus.capability-requirement.v1");
        hasher.uuid(self.id.as_uuid());
        hasher.text(self.slot.as_str());
        hasher.text(self.resource_kind.as_str());
        hasher.operations(&self.required_operations);
        hasher.operations(&self.optional_operations);
        hasher.boolean(self.slot_required);
        hasher.finish()
    }
}

#[derive(Deserialize)]
struct CapabilityRequirementWire {
    id: CapabilityRequirementId,
    slot: CapabilitySlotKey,
    resource_kind: CapabilityResourceKind,
    required_operations: Vec<CapabilityOperation>,
    optional_operations: Vec<CapabilityOperation>,
    slot_required: bool,
}

impl TryFrom<CapabilityRequirementWire> for CapabilityRequirement {
    type Error = CapabilityError;

    fn try_from(value: CapabilityRequirementWire) -> Result<Self, Self::Error> {
        Self::new(
            value.id,
            value.slot,
            value.resource_kind,
            value.required_operations,
            value.optional_operations,
            value.slot_required,
        )
    }
}

/// One exact selected Hephaestus resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct CapabilityResource {
    /// Controlled resource category.
    pub kind: CapabilityResourceKind,
    /// Stable exact resource identifier.
    pub id: Uuid,
}

impl CapabilityResource {
    /// Creates an exact typed resource reference.
    #[must_use]
    pub const fn new(kind: CapabilityResourceKind, id: Uuid) -> Self {
        Self { kind, id }
    }
}

/// One immutable binding from a symbolic slot to an exact resource ceiling.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CapabilityBinding {
    pub(super) id: CapabilityBindingId,
    pub(super) requirement_id: CapabilityRequirementId,
    pub(super) requirement_hash: AuthorityHash,
    pub(super) slot: CapabilitySlotKey,
    pub(super) resource: CapabilityResource,
    pub(super) granted_operations: BTreeSet<CapabilityOperation>,
}

impl CapabilityBinding {
    /// Validates a selected resource and explicit grants against a requirement.
    ///
    /// # Errors
    ///
    /// Rejects incompatible resource categories, missing required operations,
    /// undeclared operations, duplicate operations, and empty grants.
    pub fn bind(
        id: CapabilityBindingId,
        requirement: &CapabilityRequirement,
        resource: CapabilityResource,
        granted_operations: impl IntoIterator<Item = CapabilityOperation>,
    ) -> Result<Self, CapabilityError> {
        if resource.kind != requirement.resource_kind {
            return Err(CapabilityError::IncompatibleResourceKind {
                expected: requirement.resource_kind,
                actual: resource.kind,
            });
        }
        let granted_operations = operation_set(granted_operations)?;
        if granted_operations.is_empty() {
            return Err(CapabilityError::EmptyOperationSet);
        }
        validate_operations(resource.kind, &granted_operations)?;
        if let Some(operation) = requirement
            .required_operations
            .difference(&granted_operations)
            .next()
            .copied()
        {
            return Err(CapabilityError::MissingRequiredOperation(operation));
        }
        if let Some(operation) = granted_operations
            .iter()
            .find(|operation| !requirement.declares(**operation))
            .copied()
        {
            return Err(CapabilityError::UndeclaredOperation(operation));
        }
        Ok(Self {
            id,
            requirement_id: requirement.id,
            requirement_hash: requirement.normalized_hash(),
            slot: requirement.slot.clone(),
            resource,
            granted_operations,
        })
    }

    /// Returns the binding identifier.
    #[must_use]
    pub const fn id(&self) -> CapabilityBindingId {
        self.id
    }

    /// Returns the bound requirement identifier.
    #[must_use]
    pub const fn requirement_id(&self) -> CapabilityRequirementId {
        self.requirement_id
    }

    /// Returns the hash of the exact requirement used during binding.
    #[must_use]
    pub const fn requirement_hash(&self) -> AuthorityHash {
        self.requirement_hash
    }

    /// Returns the symbolic slot.
    #[must_use]
    pub const fn slot(&self) -> &CapabilitySlotKey {
        &self.slot
    }

    /// Returns the exact bound resource.
    #[must_use]
    pub const fn resource(&self) -> CapabilityResource {
        self.resource
    }

    /// Returns granted operations in normalized order.
    #[must_use]
    pub fn granted_operations(&self) -> impl ExactSizeIterator<Item = CapabilityOperation> + '_ {
        self.granted_operations.iter().copied()
    }

    /// Returns whether this immutable binding grants an operation.
    #[must_use]
    pub fn grants(&self, operation: CapabilityOperation) -> bool {
        self.granted_operations.contains(&operation)
    }

    /// Returns a stable hash of the normalized exact binding.
    #[must_use]
    pub fn normalized_hash(&self) -> AuthorityHash {
        let mut hasher = CanonicalHasher::new(b"hephaestus.capability-binding.v1");
        hasher.uuid(self.id.as_uuid());
        hasher.uuid(self.requirement_id.as_uuid());
        hasher.bytes(self.requirement_hash.as_bytes());
        hasher.text(self.slot.as_str());
        hasher.text(self.resource.kind.as_str());
        hasher.uuid(self.resource.id);
        hasher.operations(&self.granted_operations);
        hasher.finish()
    }
}
