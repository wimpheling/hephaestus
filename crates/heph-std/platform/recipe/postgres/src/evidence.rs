use std::collections::BTreeMap;

use capability_domain::{CapabilityRequirement, CapabilitySlotKey};
use recipe_domain::{ExternalVolume, ReleaseCatalogEntry, ReleasePin};
use release_domain::ParameterDeclaration;
use runtime_types::VolumeId;
use serde::{Deserialize, Serialize};
use volume_domain::VolumeSlotDeclaration;

/// Private historical evidence, written only from the transaction's SQL catalog.
/// Deserialization remains untrusted: hydration resolves and checks every hash.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdmissionEvidence {
    pub catalog: Vec<CatalogEvidence>,
    pub external: BTreeMap<CapabilitySlotKey, ExternalEvidence>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogEvidence {
    pub pin: ReleasePin,
    pub published: bool,
    pub parameters: Vec<ParameterDeclaration>,
    pub volume_slots: Vec<VolumeSlotDeclaration>,
    pub required_secret_slot_count: usize,
    pub capability_requirements: Vec<CapabilityRequirement>,
}

impl CatalogEvidence {
    pub fn view(&self) -> ReleaseCatalogEntry {
        ReleaseCatalogEntry {
            pin: self.pin,
            published: self.published,
            parameters: self.parameters.clone(),
            volume_slots: self.volume_slots.clone(),
            required_secret_slot_count: self.required_secret_slot_count,
            capability_requirements: self.capability_requirements.clone(),
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalEvidence {
    pub id: VolumeId,
    pub capacity_bytes: u64,
}

impl ExternalEvidence {
    pub const fn view(&self) -> ExternalVolume {
        ExternalVolume {
            id: self.id,
            capacity_bytes: self.capacity_bytes,
        }
    }
}
