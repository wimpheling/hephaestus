//! Read-only installed UI navigation projection.

use async_trait::async_trait;
use forge_domain::{OrganizationId, ProjectId, RepositoryId};
use identity_domain::AuthenticatedIdentity;
use release_domain::{
    UiInstallationGenerationId, UiInstallationId, UiInstallationState, UiInstallationTarget,
    ui::{UiIcon, UiKey, UiLabel, UiPresentation, UiRoutePath},
};
use thiserror::Error;

/// Default page size for installed UI navigation.
pub const DEFAULT_UI_INSTALLATION_PAGE_SIZE: i64 = 50;
/// Maximum page size for installed UI navigation.
pub const MAX_UI_INSTALLATION_PAGE_SIZE: i64 = 100;

/// Explicit target filter for an installed UI navigation request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiInstallationTargetFilter {
    /// List the organization-owned global installation for the organization.
    Global,
    /// List the project-scoped installation for one project in the organization.
    Project(ProjectId),
    /// List the repository-scoped installation for one repository in the organization.
    Repository(RepositoryId),
}

impl UiInstallationTargetFilter {
    /// Returns the stable database scope and optional target identifier.
    #[must_use]
    pub const fn scope_and_id(self) -> (&'static str, Option<uuid::Uuid>) {
        match self {
            Self::Global => ("global", None),
            Self::Project(id) => ("project", Some(id.as_uuid())),
            Self::Repository(id) => ("repository", Some(id.as_uuid())),
        }
    }
}

/// Cursor page for installed UI navigation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UiInstallationPage {
    /// Number of rows requested.
    pub size: i64,
    /// The last installation identity from the preceding page.
    pub after: Option<UiInstallationId>,
}

impl Default for UiInstallationPage {
    fn default() -> Self {
        Self {
            size: DEFAULT_UI_INSTALLATION_PAGE_SIZE,
            after: None,
        }
    }
}

/// Explicit organization and target navigation request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListUiInstallations {
    /// Organization boundary required for every request.
    pub organization_id: OrganizationId,
    /// One exact global, project, or repository target.
    pub target: UiInstallationTargetFilter,
    /// Bounded stable cursor page.
    pub page: UiInstallationPage,
}

/// Safe shell metadata for one currently retained UI installation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiInstallationNavigation {
    /// Stable installation identity.
    pub installation_id: UiInstallationId,
    /// Immutable generation currently selected by the installation.
    pub generation_id: UiInstallationGenerationId,
    /// Organization boundary resolved from the owner target.
    pub organization_id: OrganizationId,
    /// Exact owner target.
    pub target: UiInstallationTarget,
    /// Retained lifecycle state.
    pub lifecycle: UiInstallationState,
    /// Published source release identity.
    pub release_id: release_domain::ReleaseId,
    /// Published UI declaration key.
    pub ui_key: UiKey,
    /// Host-shell label.
    pub label: UiLabel,
    /// Host-shell icon family.
    pub icon: UiIcon,
    /// Host-shell presentation mode.
    pub presentation: UiPresentation,
    /// Platform-relative route base.
    pub route_base: UiRoutePath,
    /// Published content kind visible to the shell.
    pub content_kind: UiInstallationContentKind,
    /// Whether current source `CanUse` and all current binding permissions allow launch.
    pub launchable: bool,
}

/// Published content kind projected to the UI shell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiInstallationContentKind {
    /// Immutable artifact-backed UI.
    Static,
    /// UI backed by an immutable managed service binding.
    ManagedService,
}

/// One bounded navigation page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiInstallationNavigationPage {
    /// Safe shell projections in stable creation order.
    pub installations: Vec<UiInstallationNavigation>,
    /// Cursor for the next page.
    pub next: Option<UiInstallationId>,
}

/// Failures exposed by the read-only navigation port.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum UiInstallationNavigationError {
    /// The caller supplied an invalid page size.
    #[error("UI installation navigation page is invalid")]
    InvalidPage,
    /// Immutable database data does not satisfy the domain contract.
    #[error("stored UI installation navigation data is invalid")]
    InvalidStoredData,
    /// The authorized read transaction could not complete.
    #[error("UI installation navigation is unavailable")]
    Unavailable,
}

/// Authorized read port for installed UI navigation.
#[async_trait]
pub trait UiInstallationNavigator: Send + Sync {
    /// Lists safe shell metadata under one explicit organization and target.
    ///
    /// # Errors
    ///
    /// Returns [`UiInstallationNavigationError::InvalidPage`] for an invalid
    /// page, [`UiInstallationNavigationError::InvalidStoredData`] for invalid
    /// immutable publication data, or `Unavailable` for a failed read
    /// transaction.
    async fn list_ui_installations(
        &self,
        identity: &AuthenticatedIdentity,
        request: ListUiInstallations,
    ) -> Result<UiInstallationNavigationPage, UiInstallationNavigationError>;
}
