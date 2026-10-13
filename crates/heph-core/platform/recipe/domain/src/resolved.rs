use std::collections::BTreeMap;

use capability_domain::{CapabilityRequirement, CapabilitySlotKey};
use release_domain::{ContentHash, ParameterName, ParameterValue, ReleaseVersion};
use runtime_types::VolumeId;
use serde::Serialize;
use volume_domain::VolumeSlotDeclaration;

use crate::{RecipeVolumeBinding, ReleasePin, RemovalPolicy, ValueReference};

/// Fully resolved intent using an authoritative catalog; not an installation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResolvedRecipe {
    pub(super) contract_version: u32,
    pub(super) recipe_id: CapabilitySlotKey,
    pub(super) recipe_version: ReleaseVersion,
    pub(super) declaration_hash: ContentHash,
    pub(super) inputs: BTreeMap<ParameterName, ParameterValue>,
    pub(super) resources: BTreeMap<CapabilitySlotKey, ResolvedResource>,
    pub(super) execution_order: Vec<CapabilitySlotKey>,
    pub(super) outputs: BTreeMap<CapabilitySlotKey, ValueReference>,
}

impl ResolvedRecipe {
    /// Returns the exact resolved contract version.
    #[must_use]
    pub const fn contract_version(&self) -> u32 {
        self.contract_version
    }

    /// Returns the immutable recipe identity.
    #[must_use]
    pub const fn recipe_id(&self) -> &CapabilitySlotKey {
        &self.recipe_id
    }

    /// Returns the immutable recipe version.
    #[must_use]
    pub const fn recipe_version(&self) -> &ReleaseVersion {
        &self.recipe_version
    }

    /// Returns the declaration hash from which this snapshot was resolved.
    #[must_use]
    pub const fn declaration_hash(&self) -> ContentHash {
        self.declaration_hash
    }

    /// Returns deterministic resolved snapshot bytes, including supplied/defaulted inputs.
    ///
    /// # Errors
    ///
    /// Returns an error if canonical serialization unexpectedly fails.
    pub fn canonical_json(&self) -> Result<Vec<u8>, crate::RecipeError> {
        serde_json::to_vec(self).map_err(|_| crate::RecipeError::Serialization)
    }

    /// Returns the immutable resolved snapshot hash.
    ///
    /// # Errors
    ///
    /// Returns an error if canonical serialization unexpectedly fails.
    pub fn hash(&self) -> Result<ContentHash, crate::RecipeError> {
        self.canonical_json()
            .map(|bytes| ContentHash::digest(&bytes))
    }

    /// Returns exact named resources in canonical order.
    #[must_use]
    pub const fn resources(&self) -> &BTreeMap<CapabilitySlotKey, ResolvedResource> {
        &self.resources
    }

    /// Returns provider-independent dependency execution order.
    #[must_use]
    pub fn execution_order(&self) -> &[CapabilitySlotKey] {
        &self.execution_order
    }

    /// Returns resolved non-secret ordinary inputs.
    #[must_use]
    pub const fn inputs(&self) -> &BTreeMap<ParameterName, ParameterValue> {
        &self.inputs
    }

    /// Returns exact resource references or resolved ordinary output literals.
    #[must_use]
    pub const fn outputs(&self) -> &BTreeMap<CapabilitySlotKey, ValueReference> {
        &self.outputs
    }
}

/// Closed resolved resource intent; cannot be forged into a resolved recipe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ResolvedResource {
    /// Created or exact externally bound volume.
    Volume(ResolvedVolume),
    /// Published released instance intent.
    Instance(ResolvedInstance),
}

/// Resolved volume intent, with provenance separate from removal policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResolvedVolume {
    /// Named static prerequisites retained in the immutable resolved graph.
    pub dependencies: Vec<CapabilitySlotKey>,
    /// Positive bounded requested or externally observed capacity.
    pub capacity_bytes: u64,
    /// Exact externally bound identity; `None` means deployment-owned creation intent.
    pub external_id: Option<VolumeId>,
    /// Removal intent, always retention for external resources.
    pub removal: RemovalPolicy,
}

/// Resolved instance intent after release compatibility validation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResolvedInstance {
    /// Explicit and binding-derived prerequisites in canonical order.
    pub dependencies: Vec<CapabilitySlotKey>,
    /// Exact published release/export pin.
    pub release: ReleasePin,
    /// Validated and defaulted release parameter values.
    pub parameters: BTreeMap<ParameterName, ParameterValue>,
    /// Named exact volume references and released attachment scope.
    pub volume_bindings: Vec<RecipeVolumeBinding>,
    /// Complete authoritative released attachment scope included in snapshot identity.
    pub volume_slots: Vec<VolumeSlotDeclaration>,
    /// Immutable declared capability ceilings, never consumer grants.
    pub capability_requirements: Vec<CapabilityRequirement>,
    /// Requested instance removal policy.
    pub removal: RemovalPolicy,
}
