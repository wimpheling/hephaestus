//! Validated primitives for release-owned distribution UI declarations.

mod enums;
mod identifiers;
mod labels;

pub use enums::{
    UiCachePolicy, UiIcon, UiMediaType, UiPresentation, UiRepositoryGitAccess, UiScope,
};
pub use identifiers::{UiKey, UiRoutePath};
pub use labels::UiLabel;

#[cfg(test)]
#[path = "ui/tests.rs"]
mod tests;

use crate::ReleaseValueError;

/// The only UI declaration schema version currently supported.
pub const SCHEMA_VERSION: u16 = 1;

/// Validates a UI declaration schema version without accepting future shapes.
///
/// # Errors
///
/// Returns [`ReleaseValueError::UnsupportedUiSchemaVersion`] for every version
/// other than [`SCHEMA_VERSION`].
pub const fn validate_schema_version(version: u16) -> Result<(), ReleaseValueError> {
    if version == SCHEMA_VERSION {
        Ok(())
    } else {
        Err(ReleaseValueError::UnsupportedUiSchemaVersion)
    }
}
