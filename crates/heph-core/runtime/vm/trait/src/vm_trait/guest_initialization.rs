use super::{VmGuestVolume, invalid};
use crate::{VmError, VolumeAccessMode};
use serde::{Deserialize, Serialize};

/// Guest preparation explicitly selected from an immutable release contract.
/// This marker carries no attachment or application query permission.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VmVolumeInitializationPurpose {
    /// Application-owned bytes; the guest performs no database initialization.
    #[default]
    None,
    /// The frozen legacy state declaration requests the built-in state database.
    BuiltinStateSQLite,
}

impl VmVolumeInitializationPurpose {
    /// Returns whether no database preparation was requested.
    #[must_use]
    pub const fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }
}

impl VmGuestVolume {
    /// Adds explicit initialization metadata proved by the release loader.
    ///
    /// # Errors
    /// Rejects inconsistent frozen markers and any nonlegacy state mount shape.
    pub fn with_initialization_purpose(
        mut self,
        purpose: VmVolumeInitializationPurpose,
        frozen_requires_state: bool,
    ) -> Result<Self, VmError> {
        self.initialization_purpose = purpose;
        self.frozen_requires_state = frozen_requires_state;
        validate(&self)?;
        Ok(self)
    }

    /// Returns the explicit guest preparation purpose.
    #[must_use]
    pub const fn initialization_purpose(&self) -> VmVolumeInitializationPurpose {
        self.initialization_purpose
    }

    /// Returns the loader's explicit frozen legacy requirement marker.
    #[must_use]
    pub const fn frozen_requires_state(&self) -> bool {
        self.frozen_requires_state
    }
}

/// Rechecks metadata before accepting the complete guest mount graph.
pub fn validate(volume: &VmGuestVolume) -> Result<(), VmError> {
    let valid = match volume.initialization_purpose {
        VmVolumeInitializationPurpose::None => !volume.frozen_requires_state,
        VmVolumeInitializationPurpose::BuiltinStateSQLite => {
            volume.frozen_requires_state
                && volume.slot.as_str() == "state"
                && volume.guest_path.as_str() == "/var/lib/hephaestus"
                && volume.access_mode == VolumeAccessMode::ReadWrite
        }
    };
    if !valid {
        return Err(invalid(
            "initialization purpose differs from frozen legacy state declaration",
        ));
    }
    Ok(())
}

// Serde's skip_serializing_if callback receives a reference to the field.
#[allow(clippy::trivially_copy_pass_by_ref)]
pub const fn is_false(value: &bool) -> bool {
    !*value
}
