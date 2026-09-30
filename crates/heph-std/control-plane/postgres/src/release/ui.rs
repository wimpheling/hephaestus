//! Typed, authorized inspection of immutable release UI bindings.

mod assembly;
mod queries;
mod types;

/// Maximum published UI descriptors returned for one release.
pub const MAX_RELEASE_UI_DESCRIPTORS: usize = 16;
/// Maximum static file bindings returned for one release.
pub const MAX_RELEASE_UI_STATIC_FILES: usize = 4096;
/// Maximum API bindings returned for one release.
pub const MAX_RELEASE_UI_API_BINDINGS: usize = 256;

pub(crate) use queries::load_release_ui_descriptors;
pub use types::{
    ReleaseUiApiBinding, ReleaseUiContent, ReleaseUiDescriptor, ReleaseUiStaticFile, UiCachePolicy,
};
