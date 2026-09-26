//! Public configuration, token provider, and error types for Zot.

use async_trait::async_trait;
use registry_domain::{RegistryAuthority, RegistryNamespace, RegistryValueError};
use registry_token::IssuedToken;
use reqwest::Url;
use std::{fmt, sync::Arc, time::Duration};

/// Maximum response body accepted from the private registry.
pub(super) const MAX_MANIFEST_BYTES: usize = 4 * 1024 * 1024;
/// Bounded timeout applied to each registry request.
pub(super) const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
pub(super) const OCI_INDEX_MEDIA_TYPE: &str = "application/vnd.oci.image.index.v1+json";
pub(super) const OCI_MANIFEST_MEDIA_TYPE: &str = "application/vnd.oci.image.manifest.v1+json";
pub(super) const SBOM_ARTIFACT_TYPE: &str = "application/spdx+json";
pub(super) const PROVENANCE_ARTIFACT_TYPE: &str = "application/vnd.in-toto+json";
pub(super) const SCAN_ARTIFACT_TYPE: &str = "application/vnd.hephaestus.vulnerability-scan.v1+json";
pub(super) const SIGNATURE_ARTIFACT_TYPE: &str = "application/vnd.dev.cosign.simplesigning.v1+json";

/// Issues a fresh, exact pull token for one owned repository namespace.
#[async_trait]
pub trait RegistryPullTokenProvider: Send + Sync + 'static {
    /// Returns a short-lived token whose access claim is limited to `namespace`.
    ///
    /// # Errors
    ///
    /// Returns an opaque error when live workload authority or token issuance
    /// is unavailable.
    async fn issue_pull_token(
        &self,
        namespace: &RegistryNamespace,
    ) -> Result<IssuedToken, ZotClientError>;
}

/// Fixed private-Zot client configuration.
#[derive(Clone)]
pub struct ZotClientConfig {
    pub(in crate::zot) authority: RegistryAuthority,
    pub(in crate::zot) private_origin: Url,
}

impl ZotClientConfig {
    /// Validates the configured public authority and private Zot origin.
    ///
    /// # Errors
    ///
    /// Rejects origins containing credentials, query/fragment data, or a path.
    /// Production callers should additionally constrain this endpoint through
    /// deployment network policy; local development may use loopback HTTP.
    pub fn new(authority: RegistryAuthority, private_origin: &str) -> Result<Self, ZotClientError> {
        let origin =
            Url::parse(private_origin).map_err(|_| ZotClientError::InvalidConfiguration)?;
        let valid = matches!(origin.scheme(), "http" | "https")
            && origin.host_str().is_some()
            && origin.username().is_empty()
            && origin.password().is_none()
            && origin.query().is_none()
            && origin.fragment().is_none()
            && origin.path() == "/";
        if !valid {
            return Err(ZotClientError::InvalidConfiguration);
        }
        Ok(Self {
            authority,
            private_origin: origin,
        })
    }
}

impl fmt::Debug for ZotClientConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ZotClientConfig")
            .field("authority", &self.authority)
            .field("private_origin", &"PRIVATE")
            .finish()
    }
}

/// Exact-digest Zot reconciliation adapter.
pub struct ZotHttpRegistry<T> {
    pub(super) config: ZotClientConfig,
    pub(super) client: reqwest::Client,
    pub(super) tokens: Arc<T>,
}

/// Non-sensitive exact-digest Zot client error.
#[derive(Debug, thiserror::Error)]
pub enum ZotClientError {
    /// The fixed private endpoint or HTTP client configuration is invalid.
    #[error("Zot client configuration is invalid")]
    InvalidConfiguration,
    /// A reference attempted to select a different registry authority.
    #[error("registry authority does not match the configured Zot service")]
    AuthorityMismatch,
    /// Live token issuance, Zot, or its storage is unavailable.
    #[error("Zot registry is unavailable")]
    Unavailable,
    /// Zot rejected the short-lived pull credential.
    #[error("Zot registry authorization failed")]
    Unauthorized,
    /// Zot returned an oversized manifest or referrer index.
    #[error("Zot registry response exceeded its bound")]
    ResponseTooLarge,
    /// Zot returned bytes or descriptors that do not form a valid exact graph.
    #[error("Zot registry graph is invalid")]
    InvalidGraph,
    /// A registry domain value was invalid.
    #[error("Zot registry returned an invalid domain value")]
    Domain(#[from] RegistryValueError),
}
