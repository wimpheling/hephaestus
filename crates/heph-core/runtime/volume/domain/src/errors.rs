use capability_domain::CapabilitySlotKey;
use runtime_types::VolumeId;

/// A private-volume contract failed validation before provider effects.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum VolumeContractError {
    /// The built-in legacy declaration could not satisfy domain invariants.
    #[error("invalid built-in legacy volume declaration")]
    InvalidLegacyDeclaration,
    /// A guest path was not a bounded canonical unprotected absolute path.
    #[error("invalid controlled guest mount path")]
    InvalidGuestMountPath,
    /// A minimum-capacity claim was zero or exceeded the contract ceiling.
    #[error("volume minimum capacity must be positive and at most {maximum} bytes")]
    InvalidCapacity {
        /// Contract capacity ceiling.
        maximum: u64,
    },
    /// A declaration or binding set exceeded the slot count limit.
    #[error("volume slot count exceeds {maximum}")]
    TooManySlots {
        /// Contract slot count ceiling.
        maximum: usize,
    },
    /// Two declarations or bindings used the same symbolic slot.
    #[error("duplicate volume slot {0}")]
    DuplicateSlot(CapabilitySlotKey),
    /// Two mount paths were equal or one was nested within the other.
    #[error("volume guest mount paths overlap")]
    OverlappingMountPaths,
    /// A binding supplied a slot absent from the release declaration.
    #[error("unexpected volume binding for slot {0}")]
    UnexpectedBinding(CapabilitySlotKey),
    /// A required slot did not receive an exact resource binding.
    #[error("required volume slot {0} is unbound")]
    MissingRequiredBinding(CapabilitySlotKey),
    /// A binding mode differed from the declared runtime ceiling.
    #[error("volume access mode differs from declaration for slot {0}")]
    AccessModeMismatch(CapabilitySlotKey),
    /// One resource was selected for more than one slot.
    #[error("volume {0} is bound to more than one slot")]
    ReusedVolume(VolumeId),
}
