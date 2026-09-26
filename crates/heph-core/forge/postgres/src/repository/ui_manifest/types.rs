use agent_config::ui::RepositoryUisConfig;
use agent_config::{ConfigHash, Diagnostic, RepositoryGatewaysConfig};

/// The source-tree entry kind observed for the UI manifest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiManifestEntryKind {
    /// A regular or executable Git blob.
    Regular,
    /// A Git symbolic link.
    Symlink,
    /// A Git tree.
    Tree,
    /// A Git submodule entry.
    Gitlink,
}

/// Whether bounded source inspection accepted the declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiManifestStatus {
    /// The UI and every required gateway declaration are valid.
    Valid,
    /// The source was present but invalid; diagnostics are redacted and bounded.
    Invalid,
}

/// Bounded UI source metadata and the validated typed representation.
///
/// Typed configs and normalized hashes are authoritative only for a fully
/// valid result.  Observed gateway OIDs, sizes, and source hashes may remain
/// available on invalid results for diagnostics.  Source bytes are
/// deliberately absent from this type.
#[derive(Debug, Clone, PartialEq)]
pub struct UiManifestInspection {
    pub status: UiManifestStatus,
    pub requires_gateways: bool,
    pub entry_kind: UiManifestEntryKind,
    pub object_id: gix::ObjectId,
    pub actual_size: Option<u64>,
    pub source_hash: Option<ConfigHash>,
    pub normalized_hash: Option<ConfigHash>,
    pub config: Option<RepositoryUisConfig>,
    pub gateway_object_id: Option<gix::ObjectId>,
    pub gateway_actual_size: Option<u64>,
    pub gateway_source_hash: Option<ConfigHash>,
    pub gateway_normalized_hash: Option<ConfigHash>,
    pub gateway_config: Option<RepositoryGatewaysConfig>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct UiManifestLocation {
    pub(super) entry_kind: UiManifestEntryKind,
    pub(super) object_id: gix::ObjectId,
    pub(super) actual_size: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct GatewayManifestInspection {
    pub(super) object_id: Option<gix::ObjectId>,
    pub(super) actual_size: Option<u64>,
    pub(super) source_hash: Option<ConfigHash>,
    pub(super) normalized_hash: Option<ConfigHash>,
    pub(super) config: Option<RepositoryGatewaysConfig>,
    pub(super) diagnostics: Vec<Diagnostic>,
}
