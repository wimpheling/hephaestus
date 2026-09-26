use release_domain::ui::{
    UiIcon, UiKey, UiLabel, UiMediaType, UiPresentation, UiRepositoryGitAccess, UiRoutePath,
    UiScope,
};
use sqlx::FromRow;
use uuid::Uuid;

/// One immutable release UI descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseUiDescriptor {
    /// Stable UI key.
    pub key: UiKey,
    /// Installation scope.
    pub scope: UiScope,
    /// Host-shell label.
    pub label: UiLabel,
    /// Host-shell icon.
    pub icon: UiIcon,
    /// Initial host presentation.
    pub presentation: UiPresentation,
    /// Platform-relative route base.
    pub route_base: UiRoutePath,
    /// Relative content entrypoint.
    pub entrypoint: UiRoutePath,
    /// Version of the published UI kit contract.
    pub ui_kit_version: u16,
    /// Browser/intermediary cache policy.
    pub cache: UiCachePolicy,
    /// Explicit generic repository Git authority.
    pub repository_git_access: UiRepositoryGitAccess,
    /// Immutable content binding.
    pub content: ReleaseUiContent,
    /// Explicit gateway API bindings.
    pub apis: Vec<ReleaseUiApiBinding>,
}

/// Cache policy stored for a published release UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiCachePolicy {
    /// Do not permit browser or intermediary caching.
    NoStore,
}

/// Immutable content binding for a release UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReleaseUiContent {
    /// Static files bound to exact release artifact IDs.
    Static {
        /// Route-to-artifact bindings.
        files: Vec<ReleaseUiStaticFile>,
    },
    /// Managed service bound to one exact release agent ID.
    ManagedService {
        /// Repository gateway name.
        gateway_name: String,
        /// Absolute gateway route.
        route: String,
        /// Exact release agent identity.
        release_agent_id: Uuid,
    },
}

/// One immutable static route binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseUiStaticFile {
    /// UI-relative route.
    pub route: UiRoutePath,
    /// Exact release artifact identity.
    pub artifact_id: Uuid,
    /// Explicit artifact MIME type.
    pub media_type: UiMediaType,
}

/// One immutable gateway API binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseUiApiBinding {
    /// UI-local API key.
    pub key: UiKey,
    /// Repository gateway name.
    pub gateway_name: String,
    /// Exact HTTP method.
    pub method: String,
    /// Absolute gateway route.
    pub route: String,
    /// Exact release agent identity.
    pub release_agent_id: Uuid,
}

#[derive(Debug, FromRow)]
pub(super) struct DescriptorRow {
    pub(super) ui_key: String,
    pub(super) scope: String,
    pub(super) label: String,
    pub(super) icon: String,
    pub(super) presentation: String,
    pub(super) route_base: String,
    pub(super) entrypoint: String,
    pub(super) ui_kit_version: i32,
    pub(super) cache: String,
    pub(super) repository_git_access: String,
    pub(super) content_kind: String,
}

#[derive(Debug, FromRow)]
pub(super) struct StaticFileRow {
    pub(super) ui_key: String,
    pub(super) route: String,
    pub(super) artifact_id: Uuid,
    pub(super) artifact_kind: String,
    pub(super) artifact_media_type: String,
}

#[derive(Debug, FromRow)]
pub(super) struct ManagedServiceRow {
    pub(super) ui_key: String,
    pub(super) gateway_name: String,
    pub(super) route: String,
    pub(super) release_agent_id: Uuid,
}

#[derive(Debug, FromRow)]
pub(super) struct ApiBindingRow {
    pub(super) ui_key: String,
    pub(super) api_key: String,
    pub(super) gateway_name: String,
    pub(super) method: String,
    pub(super) route: String,
    pub(super) release_agent_id: Uuid,
}
