//! Pure resolution of static UI declarations to immutable release artifacts.
//!
//! This module deliberately has no database or storage access. Publication
//! supplies the already imported artifact identities and persists the result.

mod model;
mod resolve;

#[cfg(test)]
#[path = "static_resolution/tests.rs"]
mod tests;

pub use model::{
    MAX_STATIC_UI_FILE_BYTES, MAX_STATIC_UI_TOTAL_BYTES, ResolvedStaticFile, ResolvedStaticUi,
    ResolvedStaticUis, StaticArtifactCandidate, StaticResolutionError,
};
pub use resolve::resolve_static_uis;
