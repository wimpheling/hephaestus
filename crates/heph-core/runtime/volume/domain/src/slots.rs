use capability_domain::CapabilitySlotKey;
use runtime_types::VolumeId;
use serde::{Deserialize, Serialize};

use crate::{GuestMountPath, MAX_VOLUME_CAPACITY_BYTES, VolumeContractError};

/// Exact file attachment mode declared for a consumer.
///
/// This ceiling does not establish per-query or per-table database permissions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VolumeAccessMode {
    /// The consumer may read mounted bytes.
    ReadOnly,
    /// The consumer may read and write mounted bytes.
    ReadWrite,
}

/// One validated named private-volume requirement in a release.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "VolumeSlotDeclarationWire")]
pub struct VolumeSlotDeclaration {
    slot: CapabilitySlotKey,
    guest_path: GuestMountPath,
    access_mode: VolumeAccessMode,
    required: bool,
    minimum_capacity_bytes: u64,
}

impl VolumeSlotDeclaration {
    /// Constructs a declaration with a positive bounded minimum capacity.
    ///
    /// # Errors
    ///
    /// Rejects zero capacity and capacity above [`MAX_VOLUME_CAPACITY_BYTES`].
    /// Validate a complete release with [`crate::validate_volume_slots`].
    pub fn new(
        slot: CapabilitySlotKey,
        guest_path: GuestMountPath,
        access_mode: VolumeAccessMode,
        required: bool,
        minimum_capacity_bytes: u64,
    ) -> Result<Self, VolumeContractError> {
        if minimum_capacity_bytes == 0 || minimum_capacity_bytes > MAX_VOLUME_CAPACITY_BYTES {
            return Err(VolumeContractError::InvalidCapacity {
                maximum: MAX_VOLUME_CAPACITY_BYTES,
            });
        }
        Ok(Self {
            slot,
            guest_path,
            access_mode,
            required,
            minimum_capacity_bytes,
        })
    }

    /// Returns the symbolic release slot.
    #[must_use]
    pub const fn slot(&self) -> &CapabilitySlotKey {
        &self.slot
    }

    /// Returns the controlled guest mount path.
    #[must_use]
    pub const fn guest_path(&self) -> &GuestMountPath {
        &self.guest_path
    }

    /// Returns the exact required consumer attachment mode.
    #[must_use]
    pub const fn access_mode(&self) -> VolumeAccessMode {
        self.access_mode
    }

    /// Returns whether this slot must receive a binding.
    #[must_use]
    pub const fn required(&self) -> bool {
        self.required
    }

    /// Returns the minimum capacity claim in bytes.
    #[must_use]
    pub const fn minimum_capacity_bytes(&self) -> u64 {
        self.minimum_capacity_bytes
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VolumeSlotDeclarationWire {
    slot: CapabilitySlotKey,
    guest_path: GuestMountPath,
    access_mode: VolumeAccessMode,
    required: bool,
    minimum_capacity_bytes: u64,
}

impl TryFrom<VolumeSlotDeclarationWire> for VolumeSlotDeclaration {
    type Error = VolumeContractError;

    fn try_from(value: VolumeSlotDeclarationWire) -> Result<Self, Self::Error> {
        Self::new(
            value.slot,
            value.guest_path,
            value.access_mode,
            value.required,
            value.minimum_capacity_bytes,
        )
    }
}

/// One exact stable resource selected for a named volume slot.
///
/// A binding is a declaration and never confers live authorization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VolumeSlotBinding {
    slot: CapabilitySlotKey,
    volume_id: VolumeId,
    access_mode: VolumeAccessMode,
}

impl VolumeSlotBinding {
    /// Constructs an exact binding to validate against its release declaration.
    #[must_use]
    pub const fn new(
        slot: CapabilitySlotKey,
        volume_id: VolumeId,
        access_mode: VolumeAccessMode,
    ) -> Self {
        Self {
            slot,
            volume_id,
            access_mode,
        }
    }

    /// Returns the symbolic release slot.
    #[must_use]
    pub const fn slot(&self) -> &CapabilitySlotKey {
        &self.slot
    }

    /// Returns the selected stable private-volume identifier.
    #[must_use]
    pub const fn volume_id(&self) -> VolumeId {
        self.volume_id
    }

    /// Returns the exact consumer attachment mode.
    #[must_use]
    pub const fn access_mode(&self) -> VolumeAccessMode {
        self.access_mode
    }
}
