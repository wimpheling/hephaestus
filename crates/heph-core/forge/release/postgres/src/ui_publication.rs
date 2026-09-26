//! Release-publication preparation for repository UIs.
//!
//! This module loads the immutable UI capture attached to one exact build
//! request, validates its stored canonical hashes, resolves references to
//! caller-supplied release artifact and agent identities, and persists the
//! resulting release-owned UI bindings in the caller's publication transaction.

use super::ReleaseServiceError;

mod load;
mod model;
mod persist;

use model::UiCaptureRow;

const fn invalid_ui_storage() -> ReleaseServiceError {
    ReleaseServiceError::InvalidStoredData
}

pub use load::load_ui_publication;
pub use model::{ResolvedUiPublication, UiPublicationCandidates};
pub use persist::persist_ui_publication;

#[cfg(test)]
mod tests {
    #[path = "common.rs"]
    mod common;
    #[path = "real.rs"]
    mod real;
    #[path = "unit.rs"]
    mod unit;
}
