//! Public models used while resolving release UI publication inputs.

use agent_config::ui::RepositoryUisConfig;
use agent_config::ui::gateway_resolution::{ReleaseAgentBinding, ResolvedGatewayUis};
use agent_config::ui::static_resolution::{ResolvedStaticUis, StaticArtifactCandidate};
use forge_domain::RepositoryId;
use serde_json::Value;
use sqlx::FromRow;
use uuid::Uuid;

/// Inputs supplied by the trusted build importer for one exact release.
#[derive(Clone, Copy)]
pub struct UiPublicationCandidates<'a> {
    /// Exact repository owning the locked build request.
    pub repository_id: RepositoryId,
    /// Base build-definition hash derived from the loaded agent configuration.
    ///
    /// Legacy configurations may omit a build declaration. This remains
    /// absent for a legacy build with no UI link, but is required once a
    /// linked UI capture reaches identity verification.
    pub base_build_definition_hash: Option<[u8; 32]>,
    /// Static artifact rows imported for the release candidate.
    pub static_artifacts: &'a [StaticArtifactCandidate],
    /// Exact exported release-agent key/ID bindings for this release.
    pub release_agents: &'a [ReleaseAgentBinding],
}

/// Fully validated and identity-resolved UI publication input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedUiPublication {
    /// Immutable source-capture row to link from the release.
    pub source_manifest_revision_id: Uuid,
    /// Typed, normalized UI configuration.
    pub config: RepositoryUisConfig,
    /// Hash retained by the source-capture row.
    pub normalized_ui_hash: [u8; 32],
    /// Canonical gateway hash when the UI references gateways.
    pub normalized_gateway_hash: Option<[u8; 32]>,
    /// Static routes resolved to exact release artifact IDs.
    pub static_uis: ResolvedStaticUis,
    /// Managed/API routes resolved to exact release-agent IDs.
    pub gateway_uis: ResolvedGatewayUis,
}

#[derive(Debug, FromRow)]
pub(super) struct UiCaptureRow {
    pub(super) request_repository_id: Uuid,
    pub(super) request_source_commit: String,
    pub(super) request_build_definition_hash: Vec<u8>,
    pub(super) link_repository_id: Option<Uuid>,
    pub(super) link_source_commit: Option<String>,
    pub(super) source_manifest_revision_id: Option<Uuid>,
    pub(super) capture_repository_id: Option<Uuid>,
    pub(super) capture_source_commit: Option<String>,
    pub(super) capture_status: Option<String>,
    pub(super) normalized_ui_config: Option<Value>,
    pub(super) normalized_ui_hash: Option<Vec<u8>>,
    pub(super) requires_gateways: Option<bool>,
    pub(super) normalized_gateway_config: Option<Value>,
    pub(super) normalized_gateway_hash: Option<Vec<u8>>,
}
