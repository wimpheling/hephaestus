//! Explicit trusted selection for fresh Run producers.

/// Constructor configuration for newly admitted execution only.
///
/// This choice grants no source, delegation or physical ownership authority.
/// Historical storage classification cannot be selected or promoted here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FreshRunExecutionProfile {
    /// Preserve scalar execution with exact released legacy producer pins.
    LegacyScalar,
    /// Require an owned VM plan and canonical admission and cleanup evidence.
    OwnedCanonical,
}

impl FreshRunExecutionProfile {
    /// Returns the immutable storage label for an explicit fresh admission.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LegacyScalar => "legacy_scalar",
            Self::OwnedCanonical => "owned_canonical",
        }
    }
}
