//! Validated private-volume declarations and exact immutable release bindings.
//!
//! These declarations bound runtime attachment modes; they never confer
//! lifecycle authority or enforce permissions on individual database queries.

mod creation_identity;
mod errors;
mod grants;
mod legacy;
mod owned_backing;
mod partial_retention;
mod path;
mod selection;
mod selection_identity;
mod slots;
mod validation;

pub use creation_identity::{
    VolumeCreationIdentity, VolumeCreationIdentityError, VolumeCreationIdentityWire,
    VolumeCreationOperationId, VolumeCreationSealId, VolumeOwnershipScopeId,
};
pub use errors::VolumeContractError;
pub use grants::{VolumeMountGrantId, VolumeMountRevocationId, VolumeMountScope};
pub use legacy::{
    LEGACY_STATE_VOLUME_GUEST_PATH, LEGACY_STATE_VOLUME_SLOT, effective_volume_slots,
};
pub use owned_backing::{OwnedBackingPhase, VolumeRootNamespaceId};
pub use partial_retention::{
    OwnedPartialBirthObservation, OwnedPartialBirthObservationWire, OwnedPartialBirthPhase,
    OwnedPartialRetentionReceiptId, PartialRetentionContractError,
};
pub use path::GuestMountPath;
pub use selection::{RunVolumeSelection, RunVolumeSelections, VolumeSelectionOrigin};
pub use selection_identity::RunVolumeIdentity;
pub use slots::{VolumeAccessMode, VolumeSlotBinding, VolumeSlotDeclaration};
pub use validation::{validate_volume_bindings, validate_volume_slots};

/// Largest minimum-capacity claim accepted by the version-one slot contract.
///
/// Providers may impose narrower limits. This ceiling is sixteen tebibytes.
pub const MAX_VOLUME_CAPACITY_BYTES: u64 = 16 * 1024 * 1024 * 1024 * 1024;
/// Maximum number of named volume slots in one release contract.
pub const MAX_VOLUME_SLOTS: usize = 32;
/// Maximum UTF-8 byte length of a controlled guest mount path.
pub const MAX_GUEST_MOUNT_PATH_BYTES: usize = 256;

#[cfg(test)]
mod legacy_tests;
#[cfg(test)]
mod tests;
