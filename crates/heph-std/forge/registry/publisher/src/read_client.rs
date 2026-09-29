use registry_domain::{OciDescriptor, OciMediaType, Sha256Digest};
use registry_token::BearerToken;
use reqwest::{
    StatusCode, Url,
    blocking::{Client as HttpClient, Response as HttpResponse},
    header,
};

use super::{
    constants::REGISTRY_REQUEST_TIMEOUT, errors::RegistryReadError, remote::bounded_response,
};

/// Reads the immutable OCI Distribution data required to verify a publication.
///
/// Implementations receive only an already scoped bearer token and must not
/// follow redirects or expose that token in errors.
pub trait RegistryReadClient: Send + Sync {
    /// Reads the descriptor returned for an exact manifest digest.
    ///
    /// # Errors
    ///
    /// Returns an opaque error when the registry rejects the scoped token or
    /// cannot return a bounded valid response.
    fn manifest_descriptor(
        &self,
        namespace: &str,
        digest: &Sha256Digest,
        token: &BearerToken,
    ) -> Result<OciDescriptor, RegistryReadError>;

    /// Reads the raw bytes for an exact manifest digest.
    ///
    /// # Errors
    ///
    /// Returns an opaque error when the registry rejects the scoped token or
    /// cannot return a bounded valid response.
    fn manifest_bytes(
        &self,
        namespace: &str,
        digest: &Sha256Digest,
        token: &BearerToken,
    ) -> Result<Vec<u8>, RegistryReadError>;

    /// Reads the raw OCI referrers index for an exact subject digest.
    ///
    /// # Errors
    ///
    /// Returns an opaque error when the registry rejects the scoped token or
    /// cannot return a bounded valid response.
    fn referrers_bytes(
        &self,
        namespace: &str,
        subject: &Sha256Digest,
        token: &BearerToken,
    ) -> Result<Vec<u8>, RegistryReadError>;
}

/// Redirect free synchronous OCI Distribution client for bounded read back.
pub struct HttpRegistryReadClient {
    origin: Url,
    client: HttpClient,
}

impl HttpRegistryReadClient {
    const fn new(origin: Url, client: HttpClient) -> Self {
        Self { origin, client }
    }

    fn request(
        &self,
        namespace: &str,
        suffix: &str,
        token: &BearerToken,
        head: bool,
    ) -> Result<HttpResponse, RegistryReadError> {
        let url = self
            .origin
            .join(&format!("v2/{namespace}/{suffix}"))
            .map_err(|_| RegistryReadError::Failed)?;
        let request = if head {
            self.client.head(url)
        } else {
            self.client.get(url)
        };
        let response = request
            .bearer_auth(token.as_str())
            .header(
                header::ACCEPT,
                "application/vnd.oci.image.index.v1+json, application/vnd.oci.image.manifest.v1+json, application/vnd.oci.image.referrers.v1+json",
            )
            .send()
            .map_err(|_| RegistryReadError::Failed)?;
        if response.status() == StatusCode::UNAUTHORIZED
            || response.status() == StatusCode::FORBIDDEN
        {
            return Err(RegistryReadError::AuthenticationFailed);
        }
        response
            .status()
            .is_success()
            .then_some(response)
            .ok_or(RegistryReadError::Failed)
    }
}

impl RegistryReadClient for HttpRegistryReadClient {
    fn manifest_descriptor(
        &self,
        namespace: &str,
        digest: &Sha256Digest,
        token: &BearerToken,
    ) -> Result<OciDescriptor, RegistryReadError> {
        let response = self.request(namespace, &format!("manifests/{digest}"), token, true)?;
        let media_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(';').next())
            .ok_or(RegistryReadError::Failed)?;
        let digest = response
            .headers()
            .get("docker-content-digest")
            .and_then(|value| value.to_str().ok())
            .ok_or(RegistryReadError::Failed)?;
        let size = response
            .headers()
            .get(header::CONTENT_LENGTH)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok())
            .ok_or(RegistryReadError::Failed)?;
        OciDescriptor::new(
            Sha256Digest::parse(digest.to_owned()).map_err(|_| RegistryReadError::Failed)?,
            size,
            OciMediaType::parse(media_type.to_owned()).map_err(|_| RegistryReadError::Failed)?,
        )
        .map_err(|_| RegistryReadError::Failed)
    }

    fn manifest_bytes(
        &self,
        namespace: &str,
        digest: &Sha256Digest,
        token: &BearerToken,
    ) -> Result<Vec<u8>, RegistryReadError> {
        bounded_response(self.request(namespace, &format!("manifests/{digest}"), token, false)?)
    }

    fn referrers_bytes(
        &self,
        namespace: &str,
        subject: &Sha256Digest,
        token: &BearerToken,
    ) -> Result<Vec<u8>, RegistryReadError> {
        bounded_response(self.request(namespace, &format!("referrers/{subject}"), token, false)?)
    }
}

/// Defers construction of Reqwest's synchronous client until the
/// host-controlled publication/read-back boundary is entered.
///
/// `reqwest::blocking::Client` owns a small Tokio runtime and therefore must
/// not be constructed while the daemon is configuring itself on a Tokio
/// worker. Callers run publication through their existing blocking boundary,
/// so each bounded read creates and drops its client there.
#[derive(Clone)]
pub struct LazyHttpRegistryReadClient {
    origin: Url,
}

impl LazyHttpRegistryReadClient {
    pub(super) const fn new(origin: Url) -> Self {
        Self { origin }
    }

    fn reader(&self) -> Result<HttpRegistryReadClient, RegistryReadError> {
        let client = HttpClient::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(REGISTRY_REQUEST_TIMEOUT)
            .build()
            .map_err(|_| RegistryReadError::Failed)?;
        Ok(HttpRegistryReadClient::new(self.origin.clone(), client))
    }
}

impl RegistryReadClient for LazyHttpRegistryReadClient {
    fn manifest_descriptor(
        &self,
        namespace: &str,
        digest: &Sha256Digest,
        token: &BearerToken,
    ) -> Result<OciDescriptor, RegistryReadError> {
        self.reader()?.manifest_descriptor(namespace, digest, token)
    }

    fn manifest_bytes(
        &self,
        namespace: &str,
        digest: &Sha256Digest,
        token: &BearerToken,
    ) -> Result<Vec<u8>, RegistryReadError> {
        self.reader()?.manifest_bytes(namespace, digest, token)
    }

    fn referrers_bytes(
        &self,
        namespace: &str,
        subject: &Sha256Digest,
        token: &BearerToken,
    ) -> Result<Vec<u8>, RegistryReadError> {
        self.reader()?.referrers_bytes(namespace, subject, token)
    }
}
