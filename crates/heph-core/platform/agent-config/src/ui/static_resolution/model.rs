//! Static UI resolution models and redacted diagnostics.

use release_domain::ui::{UiKey, UiMediaType, UiRoutePath};
use release_domain::{ArtifactKind, ArtifactPath, ReleaseArtifactId};
use std::fmt;

/// Maximum size of one referenced static UI artifact.
pub const MAX_STATIC_UI_FILE_BYTES: u64 = 16 * 1024 * 1024;
/// Maximum size of distinct referenced static UI artifacts in one release.
pub const MAX_STATIC_UI_TOTAL_BYTES: u64 = 64 * 1024 * 1024;

/// One trusted artifact candidate supplied by release publication.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaticArtifactCandidate {
    /// Normalized release-relative artifact path.
    pub path: ArtifactPath,
    /// Immutable release artifact identity.
    pub id: ReleaseArtifactId,
    /// Imported artifact kind.
    pub kind: ArtifactKind,
    /// MIME type recorded by the build output declaration.
    pub media_type: String,
    /// Exact immutable artifact byte length.
    pub size_bytes: u64,
}

/// One resolved static file binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedStaticFile {
    /// UI-relative route.
    pub route: UiRoutePath,
    /// Exact immutable release artifact identity.
    pub artifact_id: ReleaseArtifactId,
    /// Validated UI MIME type.
    pub media_type: UiMediaType,
}

/// One resolved static UI declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedStaticUi {
    /// Stable UI key.
    pub key: UiKey,
    /// Route-to-artifact bindings in manifest order.
    pub files: Vec<ResolvedStaticFile>,
}

/// All static UIs resolved from one validated repository manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedStaticUis {
    /// Static declarations in manifest order. Managed-service UIs are omitted.
    pub uis: Vec<ResolvedStaticUi>,
}

/// Redacted failure from static UI resolution.
///
/// Variants contain only bounded manifest/candidate indexes. They never echo
/// repository paths or MIME values from untrusted source or build metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StaticResolutionError {
    /// The complete UI manifest failed the existing aggregate validator.
    InvalidManifest {
        /// Number of validation diagnostics.
        diagnostic_count: usize,
    },
    /// Two candidates expose the same path.
    DuplicateCandidatePath {
        /// Index of the later candidate.
        candidate_index: usize,
        /// Index of the first candidate with this path.
        first_index: usize,
    },
    /// Two candidates expose the same immutable ID.
    DuplicateCandidateId {
        /// Index of the later candidate.
        candidate_index: usize,
        /// Index of the first candidate with this ID.
        first_index: usize,
    },
    /// A declared static file has no candidate.
    MissingArtifact {
        /// Index of the UI declaration.
        ui_index: usize,
        /// Index of the file declaration within the UI.
        file_index: usize,
    },
    /// A declared static file resolved to a non-file artifact kind.
    WrongArtifactKind {
        /// Index of the UI declaration.
        ui_index: usize,
        /// Index of the file declaration within the UI.
        file_index: usize,
        /// Index of the supplied candidate.
        candidate_index: usize,
    },
    /// The build-recorded MIME differs from the UI declaration.
    MediaTypeMismatch {
        /// Index of the UI declaration.
        ui_index: usize,
        /// Index of the file declaration within the UI.
        file_index: usize,
        /// Index of the supplied candidate.
        candidate_index: usize,
    },
    /// A referenced static artifact exceeds the per-file bound.
    StaticArtifactTooLarge {
        /// Index of the UI declaration.
        ui_index: usize,
        /// Index of the file declaration within the UI.
        file_index: usize,
        /// Index of the supplied candidate.
        candidate_index: usize,
    },
    /// Distinct referenced static artifacts exceed the release aggregate bound.
    StaticArtifactTotalTooLarge {
        /// Index of the UI declaration that crossed the bound.
        ui_index: usize,
        /// Index of the file declaration that crossed the bound.
        file_index: usize,
        /// Index of the supplied candidate that crossed the bound.
        candidate_index: usize,
    },
}

impl fmt::Display for StaticResolutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidManifest { diagnostic_count } => write!(
                formatter,
                "static UI manifest validation failed ({diagnostic_count} diagnostics)"
            ),
            Self::DuplicateCandidatePath {
                candidate_index,
                first_index,
            } => write!(
                formatter,
                "duplicate static artifact candidate path at candidates[{candidate_index}] (first at candidates[{first_index}])"
            ),
            Self::DuplicateCandidateId {
                candidate_index,
                first_index,
            } => write!(
                formatter,
                "duplicate static artifact candidate ID at candidates[{candidate_index}] (first at candidates[{first_index}])"
            ),
            Self::MissingArtifact {
                ui_index,
                file_index,
            } => write!(
                formatter,
                "static artifact is unavailable at uis[{ui_index}].content.files[{file_index}]"
            ),
            Self::WrongArtifactKind {
                ui_index,
                file_index,
                candidate_index,
            } => write!(
                formatter,
                "static artifact has an unsupported kind at uis[{ui_index}].content.files[{file_index}] (candidate {candidate_index})"
            ),
            Self::MediaTypeMismatch {
                ui_index,
                file_index,
                candidate_index,
            } => write!(
                formatter,
                "static artifact MIME does not match at uis[{ui_index}].content.files[{file_index}] (candidate {candidate_index})"
            ),
            Self::StaticArtifactTooLarge {
                ui_index,
                file_index,
                candidate_index,
            } => write!(
                formatter,
                "static artifact exceeds the per-file size bound at uis[{ui_index}].content.files[{file_index}] (candidate {candidate_index})"
            ),
            Self::StaticArtifactTotalTooLarge {
                ui_index,
                file_index,
                candidate_index,
            } => write!(
                formatter,
                "static UI artifacts exceed the aggregate size bound at uis[{ui_index}].content.files[{file_index}] (candidate {candidate_index})"
            ),
        }
    }
}

impl std::error::Error for StaticResolutionError {}
