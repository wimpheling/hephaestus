use capability_domain::{CapabilityRequirement, CapabilitySlotKey};
use release_domain::{ParameterDeclaration, ReleaseAgentId, ReleaseId};
use serde::{Deserialize, Serialize};
use volume_domain::VolumeSlotDeclaration;

/// Exact release and export identities; mutable names and tags are unsupported.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleasePin {
    /// Immutable release identity.
    pub release_id: ReleaseId,
    /// Export identity belonging to that release.
    pub release_agent_id: ReleaseAgentId,
}

/// Authoritative release view loaded by a trusted application catalog adapter.
///
/// Constructing this view does not itself verify publication or grant authority;
/// callers must derive it from authoritative records, never caller-supplied JSON.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseCatalogEntry {
    /// Exact catalog release/export association.
    pub pin: ReleasePin,
    /// Whether authoritative publication has frozen this release/export.
    pub published: bool,
    /// Released typed ordinary parameter declarations.
    pub parameters: Vec<ParameterDeclaration>,
    /// Released typed volume-slot requirements.
    pub volume_slots: Vec<VolumeSlotDeclaration>,
    /// Count derived from required released secret slots; only zero is supported.
    pub required_secret_slot_count: usize,
    /// Additional exact capability requirements, including volume attach ceilings.
    pub capability_requirements: Vec<CapabilityRequirement>,
}

pub fn slot_path(resource: &CapabilitySlotKey, slot: &CapabilitySlotKey) -> String {
    format!("resources.{resource}.volume_bindings.{slot}")
}
