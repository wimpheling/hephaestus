use async_trait::async_trait;
use identity_domain::RequestId;
use release_domain::{
    ContentHash, ReleaseArtifactId, UiInstallationGenerationId,
    ui::{UiCachePolicy, UiMediaType},
    ui_browser::UiBrowserSessionSecret,
};
use uuid::Uuid;

use crate::{UiBrowserSessionContext, ui_browser_host::UiGenerationHost};

use super::http::{UiBrowserHttpPath, UiBrowserHttpRequest, UiGatewayRequestProjection};

/// The only pre-auth host result.
///
/// The encoded generation is currently enabled and selected by its
/// installation. No route, release, organization, or label metadata is
/// returned before child-session authentication.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActiveUiGenerationHost {
    /// Exact generation encoded by the host label.
    pub generation_id: UiInstallationGenerationId,
}

/// Application-role read port for the pre-auth host existence check.
#[async_trait]
pub trait UiGenerationHostResolver: Send + Sync {
    /// Returns a match only when the generation is the current enabled one.
    async fn resolve_active_generation_host(
        &self,
        host: UiGenerationHost,
    ) -> Result<Option<ActiveUiGenerationHost>, UiHostLookupError>;
}

/// Failures from the intentionally metadata-free host lookup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum UiHostLookupError {
    /// The application-role host function or connection was unavailable.
    #[error("UI host lookup is unavailable")]
    Unavailable,
}

/// Immutable static artifact metadata needed by the local verified store.
/// `storage_key` is an internal adapter locator and must never be serialized
/// into a browser response or guest request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiStaticArtifactProjection {
    /// Published release artifact identity.
    pub artifact_id: ReleaseArtifactId,
    /// Opaque local-store object identity.
    pub storage_key: Uuid,
    /// Exact content hash recorded at publication.
    pub content_hash: ContentHash,
    /// Exact object length.
    pub size_bytes: u64,
    /// Closed published UI media type.
    pub media_type: UiMediaType,
    /// Published cache behavior.
    pub cache_policy: UiCachePolicy,
}

/// Current declaration resource selected for one authenticated request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiServingProjection {
    /// Authenticated route-base alias that must be redirected to its
    /// canonical entrypoint before the resource is requested.
    Redirect {
        /// Verified child-session and installation context for the redirect.
        context: UiBrowserSessionContext,
        /// Canonical absolute entrypoint location.
        location: UiBrowserHttpPath,
    },
    /// Static artifact to read with `LocalArtifactStore::read_verified`.
    Static {
        /// Verified child-session and installation context for the read.
        context: UiBrowserSessionContext,
        /// Immutable artifact projection selected by the verified route.
        artifact: UiStaticArtifactProjection,
    },
    /// Managed/API authority to pass to the UI gateway admission seam.
    Gateway {
        /// Verified child-session and installation context for the request.
        context: UiBrowserSessionContext,
        /// Canonical gateway route and method selected for forwarding.
        request: UiGatewayRequestProjection,
    },
}

/// App-pool boundary that classifies raw HTTP exactly once after child
/// authentication. Implementations delegate each typed candidate to the
/// existing verifier and fail closed on zero or multiple eligible matches.
#[async_trait]
pub trait UiBrowserHttpServingProjection: Send + Sync {
    /// GET: static/managed/API GET; HEAD: static-as-GET/API HEAD; other
    /// methods: exact API method only. Query is never part of matching.
    async fn authenticate_and_project_http(
        &self,
        request_id: RequestId,
        session_secret: UiBrowserSessionSecret,
        expected_generation_id: UiInstallationGenerationId,
        request: UiBrowserHttpRequest,
    ) -> Result<UiServingProjection, UiServingError>;
}

/// Redacted serving projection failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum UiServingError {
    /// Child, generation, route, method, lifecycle, or permission denied.
    #[error("UI request is unauthenticated")]
    Unauthenticated,
    /// App-pool verifier/projection failed or stored publication was invalid.
    #[error("UI serving projection is unavailable")]
    Unavailable,
}
