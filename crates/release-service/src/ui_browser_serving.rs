//! Provider-neutral UI host and resource read ports.
//!
//! Implementations belong in `release-postgres`; HTTP handlers receive typed
//! ports and projections rather than SQL pools.

use async_trait::async_trait;
use forge_domain::RepositoryId;
use gateway_domain::HttpMethod;
use identity_domain::{RequestId, UserId};
use release_domain::{
    ContentHash, ReleaseArtifactId, UiInstallationGenerationId,
    ui::{UiCachePolicy, UiMediaType},
    ui_browser::UiBrowserSessionSecret,
};
use uuid::Uuid;

use crate::{UiBrowserSessionContext, ui_browser_host::UiGenerationHost};

/// Smart-HTTP operation requested through the reserved UI Git path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiRepositoryGitOperation {
    /// Read refs or objects.
    Read,
    /// Receive a write to the repository.
    Write,
}

impl UiRepositoryGitOperation {
    /// Returns the verifier operation spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
        }
    }
}

/// Typed repository Git authority returned by the browser verifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiRepositoryGitAuthorization {
    /// Actual user selected by the live browser child session.
    pub actor_id: UserId,
    /// Exact repository bound to the UI installation.
    pub repository_id: RepositoryId,
    /// Effective approved access mode.
    pub access: release_domain::ui::UiRepositoryGitAccess,
    /// Complete child-session context used by the durable UI audit boundary.
    pub context: UiBrowserSessionContext,
}

/// Application-role port for reserved same-origin UI Git authorization.
#[async_trait]
pub trait UiBrowserRepositoryGitAuthorization: Send + Sync {
    /// Rechecks live session, installation, generation, release, and grants.
    async fn authorize_repository_git(
        &self,
        request_id: RequestId,
        session_secret: UiBrowserSessionSecret,
        expected_generation_id: UiInstallationGenerationId,
        repository_id: RepositoryId,
        operation: UiRepositoryGitOperation,
    ) -> Result<UiRepositoryGitAuthorization, UiGitAuthorizationError>;
}

/// Redacted repository Git authorization failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum UiGitAuthorizationError {
    /// The live UI child or repository grants do not authorize this operation.
    #[error("UI repository Git request is unauthorized")]
    Unauthorized,
    /// The application verifier or connection was unavailable.
    #[error("UI repository Git authorization is unavailable")]
    Unavailable,
}

/// Raw canonical absolute HTTP path used for adapter-side declaration
/// classification. Its bound covers a 256-byte route base plus a 256-byte
/// published file path and their separating slash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiBrowserHttpPath(String);

impl UiBrowserHttpPath {
    /// Parses the path grammar used by the HTTP serving boundary.
    ///
    /// # Errors
    ///
    /// Returns [`UiBrowserHttpPathError::Invalid`] when the path is not a
    /// canonical absolute UI path.
    pub fn parse(value: impl Into<String>) -> Result<Self, UiBrowserHttpPathError> {
        let value = value.into();
        let valid = (2..=514).contains(&value.len())
            && value.starts_with('/')
            && !value.ends_with('/')
            && value.bytes().all(is_ascii_unreserved_or_slash)
            && value[1..]
                .split('/')
                .all(|segment| !segment.is_empty() && segment != "." && segment != "..");
        if valid {
            Ok(Self(value))
        } else {
            Err(UiBrowserHttpPathError::Invalid)
        }
    }

    /// Returns the validated absolute path without a query or fragment.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

const fn is_ascii_unreserved_or_slash(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'/')
}

/// Invalid raw HTTP authority path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum UiBrowserHttpPathError {
    /// Path is not a canonical absolute ASCII UI path.
    #[error("invalid UI HTTP path")]
    Invalid,
}

/// Raw canonical HTTP request used for adapter-side declaration classification.
/// Query text is handled by the HTTP boundary and is excluded from this
/// authority port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiBrowserHttpRequest {
    /// Canonical method vocabulary.
    pub method: HttpMethod,
    /// Canonical absolute path without query.
    path: UiBrowserHttpPath,
}

impl UiBrowserHttpRequest {
    /// Creates one raw request after the HTTP boundary has validated its
    /// canonical path grammar.
    ///
    /// # Errors
    ///
    /// Returns [`UiBrowserHttpPathError::Invalid`] when `path` is not a
    /// canonical absolute UI path.
    pub fn new(
        method: HttpMethod,
        path: impl Into<String>,
    ) -> Result<Self, UiBrowserHttpPathError> {
        Ok(Self {
            method,
            path: UiBrowserHttpPath::parse(path)?,
        })
    }

    /// Returns the canonical method.
    #[must_use]
    pub const fn method(&self) -> HttpMethod {
        self.method
    }

    /// Returns the canonical absolute path.
    #[must_use]
    pub const fn path(&self) -> &UiBrowserHttpPath {
        &self.path
    }
}

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

/// The declaration kind selected by the raw HTTP classifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiGatewayRequestKind {
    /// A route-base/descendant request for a managed service.
    Managed,
    /// An exact declared API route and method.
    Api,
}

/// Safe managed/API request shape after child authentication.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiGatewayRequestProjection {
    /// The exact declaration kind selected by the adapter.
    pub kind: UiGatewayRequestKind,
    /// Validated raw HTTP path, including the managed route base when present.
    /// The gateway worker derives the current binding and route again.
    pub path: UiBrowserHttpPath,
    /// The canonical HTTP method selected by the declaration matcher.
    pub method: HttpMethod,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_request_keeps_query_out_of_authority_path() {
        let request = UiBrowserHttpRequest::new(HttpMethod::Get, "/docs/app.js").expect("path");
        assert_eq!(request.path().as_str(), "/docs/app.js");
    }

    #[test]
    fn raw_request_rejects_ambiguous_path_syntax() {
        for path in [
            "//docs/app.js",
            "/docs%2Fapp.js",
            "/docs/../app.js",
            "/docs/app.js?",
        ] {
            assert!(
                UiBrowserHttpRequest::new(HttpMethod::Get, path).is_err(),
                "{path}"
            );
        }
    }

    #[test]
    fn raw_request_accepts_composed_static_path_bound() {
        let path = format!("/{}", "a".repeat(513));
        assert!(UiBrowserHttpRequest::new(HttpMethod::Get, path).is_ok());
        let too_long = format!("/{}", "a".repeat(514));
        assert!(UiBrowserHttpRequest::new(HttpMethod::Get, too_long).is_err());
    }
}
