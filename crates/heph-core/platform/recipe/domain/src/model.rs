use std::collections::BTreeMap;

use capability_domain::CapabilitySlotKey;
use release_domain::{ParameterName, ParameterValue, ReleaseVersion};
use serde::{Deserialize, Serialize};
use volume_domain::{GuestMountPath, VolumeAccessMode};

use crate::{InputDeclaration, ReleasePin};

/// Untrusted recipe syntax; construct validated intent with [`crate::parse_recipe`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecipeManifest {
    /// Exact supported schema and semantics version.
    pub contract_version: u32,
    /// Stable bounded recipe identity.
    pub recipe_id: CapabilitySlotKey,
    /// Immutable version within this recipe identity.
    pub recipe_version: ReleaseVersion,
    /// Typed ordinary inputs; secret inputs are unsupported.
    #[serde(default)]
    pub inputs: Vec<InputDeclaration>,
    /// Static named resource graph.
    pub resources: Vec<ResourceDeclaration>,
    /// Explicit ordinary values or resource identity outputs.
    #[serde(default)]
    pub outputs: Vec<RecipeOutput>,
}

/// Closed first-phase resource vocabulary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ResourceDeclaration {
    /// Bounded created or explicitly external volume.
    Volume(VolumeResource),
    /// Instance using an exact published release and declared volume slots.
    Instance(InstanceResource),
}

impl ResourceDeclaration {
    /// Returns the uniquely named resource.
    #[must_use]
    pub const fn name(&self) -> &CapabilitySlotKey {
        match self {
            Self::Volume(value) => &value.name,
            Self::Instance(value) => &value.name,
        }
    }

    pub(super) fn dependencies(&self) -> &[CapabilitySlotKey] {
        match self {
            Self::Volume(value) => &value.depends_on,
            Self::Instance(value) => &value.depends_on,
        }
    }
}

/// Private-volume declaration without provider paths or handles.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VolumeResource {
    /// Unique graph name.
    pub name: CapabilitySlotKey,
    /// Creation intent or explicit external provenance.
    pub source: VolumeSource,
    /// Explicit resource dependencies.
    #[serde(default)]
    pub depends_on: Vec<CapabilitySlotKey>,
    /// Removal intent; durable data defaults to retention.
    #[serde(default)]
    pub removal: RemovalPolicy,
}

/// Volume provenance. External resource identities are supplied at resolution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum VolumeSource {
    /// Provision a new deployment-owned resource.
    Created {
        /// Positive bounded capacity in bytes from a literal or integer input.
        capacity_bytes: ValueReference,
    },
    /// Bind an explicitly external resource without transferring ownership.
    // Empty struct variants ensure Serde rejects unknown nested fields.
    External {},
}

/// Instance declaration; the recipe cannot override release executable intent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstanceResource {
    /// Unique graph name.
    pub name: CapabilitySlotKey,
    /// Exact immutable release identities, verified against the supplied catalog.
    pub release: ReleasePin,
    /// Ordinary parameters from literals or typed inputs.
    #[serde(default)]
    pub parameters: BTreeMap<ParameterName, ValueReference>,
    /// Explicit declared-slot bindings.
    #[serde(default)]
    pub volume_bindings: Vec<RecipeVolumeBinding>,
    /// Additional explicit dependencies.
    #[serde(default)]
    pub depends_on: Vec<CapabilitySlotKey>,
    /// Explicit removal policy for this deployment-owned instance.
    #[serde(default)]
    pub removal: RemovalPolicy,
}

/// Named volume reference bounded by an authoritative release slot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecipeVolumeBinding {
    /// Declared release slot name.
    pub slot: CapabilitySlotKey,
    /// Exact resource name within this recipe graph.
    pub resource: CapabilitySlotKey,
    /// Controlled guest path, which must equal the released slot's path.
    pub guest_path: GuestMountPath,
    /// Must exactly match the released slot's attachment mode.
    pub access_mode: VolumeAccessMode,
}

/// Removal intent, never permission to delete a resource.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemovalPolicy {
    /// Preserve discoverable resource ownership and durable data.
    #[default]
    Retain,
    /// Request authorized removal of a deployment-owned resource.
    Delete,
}

/// Explicit bounded reference vocabulary without expressions or interpolation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case", deny_unknown_fields)]
pub enum ValueReference {
    /// Ordinary scalar value.
    Literal {
        /// Uncoerced string, integer, or boolean.
        value: ParameterValue,
    },
    /// Named declared ordinary input.
    Input {
        /// Recipe input name.
        name: ParameterName,
    },
    /// Named resource identity; allowed only in declared outputs.
    Resource {
        /// Recipe resource name.
        name: CapabilitySlotKey,
    },
}

/// Explicit named output. Its value grants no ambient authority.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecipeOutput {
    /// Unique bounded output name.
    pub name: CapabilitySlotKey,
    /// Literal, input, or exact graph-resource reference.
    pub value: ValueReference,
}
