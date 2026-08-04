//! Authenticated, bounded Zot reads for registry reconciliation.
//!
//! The adapter addresses only an administrator-configured private Zot origin,
//! refuses redirects, requests exact digests, and validates returned bytes
//! before constructing control-plane evidence.

use async_trait::async_trait;
use registry_domain::{
    ImmutableManifestReference, OciDescriptor, OciMediaType, PlatformDescriptor, RegistryAuthority,
    RegistryNamespace, RegistryValueError, Sha256Digest, SupplyChainEvidence, SupplyChainReferrer,
    SupplyChainReferrerKind,
};
use registry_reconciler::{ReconciliationPortError, ZotInspection, ZotRegistry};
use registry_token::IssuedToken;
use reqwest::{Client, StatusCode, Url, header};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{fmt, sync::Arc, time::Duration};

const MAX_MANIFEST_BYTES: usize = 4 * 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const OCI_INDEX_MEDIA_TYPE: &str = "application/vnd.oci.image.index.v1+json";
const OCI_MANIFEST_MEDIA_TYPE: &str = "application/vnd.oci.image.manifest.v1+json";
const SBOM_ARTIFACT_TYPE: &str = "application/spdx+json";
const PROVENANCE_ARTIFACT_TYPE: &str = "application/vnd.in-toto+json";
const SCAN_ARTIFACT_TYPE: &str = "application/vnd.hephaestus.vulnerability-scan.v1+json";
const SIGNATURE_ARTIFACT_TYPE: &str = "application/vnd.dev.cosign.simplesigning.v1+json";

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
    authority: RegistryAuthority,
    private_origin: Url,
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
    config: ZotClientConfig,
    client: Client,
    tokens: Arc<T>,
}

impl<T> ZotHttpRegistry<T>
where
    T: RegistryPullTokenProvider,
{
    /// Builds a redirect-free bounded HTTP client.
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying HTTP client cannot be configured.
    pub fn new(config: ZotClientConfig, tokens: Arc<T>) -> Result<Self, ZotClientError> {
        let client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|_| ZotClientError::InvalidConfiguration)?;
        Ok(Self {
            config,
            client,
            tokens,
        })
    }

    async fn inspect_exact(
        &self,
        reference: &ImmutableManifestReference,
    ) -> Result<ZotInspection, ZotClientError> {
        if reference.authority() != &self.config.authority {
            return Err(ZotClientError::AuthorityMismatch);
        }
        let issued = self.tokens.issue_pull_token(reference.namespace()).await?;
        let token = issued.token().as_str();
        let Some(subject) = self
            .fetch_manifest(reference.namespace(), reference.digest(), token, true)
            .await?
        else {
            return Ok(ZotInspection::Missing);
        };
        if !subject.descriptor.media_type().is_image_index() {
            return Err(ZotClientError::InvalidGraph);
        }
        let platforms = subject
            .document
            .manifests
            .iter()
            .map(RemoteDescriptor::to_platform)
            .collect::<Result<Vec<_>, _>>()?;
        if platforms.is_empty() {
            return Err(ZotClientError::InvalidGraph);
        }
        let referrer_index = self
            .get_json(
                reference.namespace(),
                &format!("referrers/{}", reference.digest()),
                token,
            )
            .await?;
        let discovered: RemoteIndex =
            serde_json::from_slice(&referrer_index).map_err(|_| ZotClientError::InvalidGraph)?;
        let mut evidence = Vec::new();
        for descriptor in &discovered.manifests {
            let Some(kind) = referrer_kind(descriptor.artifact_type.as_deref()) else {
                continue;
            };
            let digest = Sha256Digest::parse(descriptor.digest.clone())?;
            let Some(remote) = self
                .fetch_manifest(reference.namespace(), &digest, token, false)
                .await?
            else {
                return Err(ZotClientError::InvalidGraph);
            };
            let expected_artifact_type = artifact_type(kind);
            if remote.document.media_type.as_deref() != Some(OCI_MANIFEST_MEDIA_TYPE)
                || remote.document.artifact_type.as_deref() != Some(expected_artifact_type)
                || remote.document.blobs.is_empty()
                || remote
                    .document
                    .subject
                    .as_ref()
                    .map(|value| value.digest.as_str())
                    != Some(reference.digest().as_str())
                || remote.descriptor != descriptor.to_domain()?
            {
                return Err(ZotClientError::InvalidGraph);
            }
            evidence.push(SupplyChainReferrer::new(
                kind,
                reference.digest().clone(),
                remote.descriptor,
                OciMediaType::parse(expected_artifact_type)?,
            ));
        }
        Ok(ZotInspection::Present {
            manifest: subject.descriptor,
            platforms,
            evidence: SupplyChainEvidence::new(reference.digest().clone(), evidence)?,
        })
    }

    async fn fetch_manifest(
        &self,
        namespace: &RegistryNamespace,
        digest: &Sha256Digest,
        token: &str,
        allow_missing: bool,
    ) -> Result<Option<FetchedManifest>, ZotClientError> {
        let url = self.url(namespace, &format!("manifests/{digest}"))?;
        let response = self
            .client
            .get(url)
            .bearer_auth(token)
            .header(
                header::ACCEPT,
                format!("{OCI_INDEX_MEDIA_TYPE}, {OCI_MANIFEST_MEDIA_TYPE}"),
            )
            .send()
            .await
            .map_err(|_| ZotClientError::Unavailable)?;
        if allow_missing && response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if response.status() != StatusCode::OK {
            return Err(status_error(response.status()));
        }
        let advertised_digest = response
            .headers()
            .get("docker-content-digest")
            .and_then(|value| value.to_str().ok())
            .ok_or(ZotClientError::InvalidGraph)?;
        if advertised_digest != digest.as_str() {
            return Err(ZotClientError::InvalidGraph);
        }
        let bytes = bounded_bytes(response).await?;
        let actual = Sha256Digest::parse(format!("sha256:{:x}", Sha256::digest(&bytes)))?;
        if &actual != digest {
            return Err(ZotClientError::InvalidGraph);
        }
        let document: RemoteManifest =
            serde_json::from_slice(&bytes).map_err(|_| ZotClientError::InvalidGraph)?;
        let media_type = OciMediaType::parse(
            document
                .media_type
                .clone()
                .ok_or(ZotClientError::InvalidGraph)?,
        )?;
        let descriptor = OciDescriptor::new(
            actual,
            u64::try_from(bytes.len()).map_err(|_| ZotClientError::ResponseTooLarge)?,
            media_type,
        )?;
        Ok(Some(FetchedManifest {
            descriptor,
            document,
        }))
    }

    async fn get_json(
        &self,
        namespace: &RegistryNamespace,
        suffix: &str,
        token: &str,
    ) -> Result<Vec<u8>, ZotClientError> {
        let response = self
            .client
            .get(self.url(namespace, suffix)?)
            .bearer_auth(token)
            .header(header::ACCEPT, OCI_INDEX_MEDIA_TYPE)
            .send()
            .await
            .map_err(|_| ZotClientError::Unavailable)?;
        if response.status() != StatusCode::OK {
            return Err(status_error(response.status()));
        }
        bounded_bytes(response).await
    }

    fn url(&self, namespace: &RegistryNamespace, suffix: &str) -> Result<Url, ZotClientError> {
        self.config
            .private_origin
            .join(&format!("v2/{}/{suffix}", namespace.as_str()))
            .map_err(|_| ZotClientError::InvalidConfiguration)
    }
}

#[async_trait]
impl<T> ZotRegistry for ZotHttpRegistry<T>
where
    T: RegistryPullTokenProvider,
{
    async fn inspect(
        &self,
        reference: &ImmutableManifestReference,
    ) -> Result<ZotInspection, ReconciliationPortError> {
        match self.inspect_exact(reference).await {
            Ok(inspection) => Ok(inspection),
            Err(
                ZotClientError::InvalidGraph
                | ZotClientError::ResponseTooLarge
                | ZotClientError::Domain(_),
            ) => Ok(ZotInspection::Invalid),
            Err(
                ZotClientError::InvalidConfiguration
                | ZotClientError::AuthorityMismatch
                | ZotClientError::Unavailable
                | ZotClientError::Unauthorized,
            ) => Err(ReconciliationPortError),
        }
    }
}

async fn bounded_bytes(response: reqwest::Response) -> Result<Vec<u8>, ZotClientError> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_MANIFEST_BYTES as u64)
    {
        return Err(ZotClientError::ResponseTooLarge);
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|_| ZotClientError::Unavailable)?;
    if bytes.is_empty() || bytes.len() > MAX_MANIFEST_BYTES {
        return Err(ZotClientError::ResponseTooLarge);
    }
    Ok(bytes.to_vec())
}

fn status_error(status: StatusCode) -> ZotClientError {
    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        ZotClientError::Unauthorized
    } else {
        ZotClientError::Unavailable
    }
}

const fn artifact_type(kind: SupplyChainReferrerKind) -> &'static str {
    match kind {
        SupplyChainReferrerKind::Sbom => SBOM_ARTIFACT_TYPE,
        SupplyChainReferrerKind::Provenance => PROVENANCE_ARTIFACT_TYPE,
        SupplyChainReferrerKind::Scan => SCAN_ARTIFACT_TYPE,
        SupplyChainReferrerKind::Signature => SIGNATURE_ARTIFACT_TYPE,
    }
}

fn referrer_kind(value: Option<&str>) -> Option<SupplyChainReferrerKind> {
    match value {
        Some(SBOM_ARTIFACT_TYPE) => Some(SupplyChainReferrerKind::Sbom),
        Some(PROVENANCE_ARTIFACT_TYPE) => Some(SupplyChainReferrerKind::Provenance),
        Some(SCAN_ARTIFACT_TYPE) => Some(SupplyChainReferrerKind::Scan),
        Some(SIGNATURE_ARTIFACT_TYPE) => Some(SupplyChainReferrerKind::Signature),
        _ => None,
    }
}

struct FetchedManifest {
    descriptor: OciDescriptor,
    document: RemoteManifest,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RemoteManifest {
    media_type: Option<String>,
    #[serde(default)]
    artifact_type: Option<String>,
    #[serde(default)]
    subject: Option<RemoteSubject>,
    #[serde(default)]
    manifests: Vec<RemoteDescriptor>,
    #[serde(default)]
    blobs: Vec<RemoteDescriptor>,
}

#[derive(Deserialize)]
struct RemoteSubject {
    digest: String,
}

#[derive(Deserialize)]
struct RemoteIndex {
    manifests: Vec<RemoteDescriptor>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RemoteDescriptor {
    media_type: String,
    digest: String,
    size: u64,
    #[serde(default)]
    artifact_type: Option<String>,
    #[serde(default)]
    platform: Option<RemotePlatform>,
}

impl RemoteDescriptor {
    fn to_domain(&self) -> Result<OciDescriptor, ZotClientError> {
        OciDescriptor::new(
            Sha256Digest::parse(self.digest.clone())?,
            self.size,
            OciMediaType::parse(self.media_type.clone())?,
        )
        .map_err(ZotClientError::Domain)
    }

    fn to_platform(&self) -> Result<PlatformDescriptor, ZotClientError> {
        let platform = self.platform.as_ref().ok_or(ZotClientError::InvalidGraph)?;
        PlatformDescriptor::new(
            self.to_domain()?,
            platform.operating_system.clone(),
            platform.architecture.clone(),
            platform.variant.clone(),
        )
        .map_err(ZotClientError::Domain)
    }
}

#[derive(Deserialize)]
struct RemotePlatform {
    #[serde(rename = "os")]
    operating_system: String,
    architecture: String,
    #[serde(default)]
    variant: Option<String>,
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Router,
        body::Body,
        extract::{Request, State},
        http::{HeaderValue, Response},
    };
    use registry_domain::{PlatformBuilderKey, RegistryOwner};
    use registry_token::{
        AuthorizationDecision, KeyId, RegistryService, RegistryTokenIssuer, RepositoryActions,
        RepositoryName, ScopeRequest, SigningKey, TokenIssuer, TokenLifetime, TokenSubject,
        UnixTimestamp,
    };
    use std::collections::BTreeMap;
    use tokio::task::JoinHandle;

    struct TestTokens {
        issuer: RegistryTokenIssuer,
    }

    #[async_trait]
    impl RegistryPullTokenProvider for TestTokens {
        async fn issue_pull_token(
            &self,
            namespace: &RegistryNamespace,
        ) -> Result<IssuedToken, ZotClientError> {
            let request = ScopeRequest::parse(
                self.issuer.service().as_str(),
                &format!("repository:{}:pull", namespace.as_str()),
            )
            .map_err(|_| ZotClientError::Unavailable)?;
            let repository = namespace
                .as_str()
                .parse::<RepositoryName>()
                .map_err(|_| ZotClientError::Unavailable)?;
            let mut decision = AuthorizationDecision::deny_all();
            decision.grant(repository, RepositoryActions::pull());
            self.issuer
                .issue(
                    "workload:reconciler"
                        .parse::<TokenSubject>()
                        .map_err(|_| ZotClientError::Unavailable)?,
                    &request,
                    &decision,
                    UnixTimestamp::new(1_700_000_000),
                )
                .map_err(|_| ZotClientError::Unavailable)
        }
    }

    #[derive(Clone)]
    struct FakeRegistry {
        routes: Arc<BTreeMap<String, FakeResponse>>,
    }

    #[derive(Clone)]
    struct FakeResponse {
        status: StatusCode,
        digest: Option<String>,
        body: Vec<u8>,
    }

    async fn fake_registry(State(state): State<FakeRegistry>, request: Request) -> Response<Body> {
        let authenticated = request
            .headers()
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.starts_with("Bearer ") && value.len() > 20);
        if !authenticated {
            return Response::builder()
                .status(StatusCode::UNAUTHORIZED)
                .body(Body::empty())
                .expect("unauthorized response");
        }
        let Some(reply) = state.routes.get(request.uri().path()) else {
            return Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Body::empty())
                .expect("not-found response");
        };
        let mut response = Response::builder().status(reply.status);
        if let Some(digest) = &reply.digest {
            response = response.header(
                "docker-content-digest",
                HeaderValue::from_str(digest).expect("digest header"),
            );
        }
        response
            .body(Body::from(reply.body.clone()))
            .expect("fake response")
    }

    async fn serve(routes: BTreeMap<String, FakeResponse>) -> (String, JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener");
        let address = listener.local_addr().expect("address");
        let application = Router::new()
            .fallback(fake_registry)
            .with_state(FakeRegistry {
                routes: Arc::new(routes),
            });
        let task = tokio::spawn(async move {
            axum::serve(listener, application)
                .await
                .expect("fake registry");
        });
        (format!("http://{address}/"), task)
    }

    fn tokens(authority: &RegistryAuthority) -> Arc<TestTokens> {
        Arc::new(TestTokens {
            issuer: RegistryTokenIssuer::new(
                "https://forge.test/v1/registry/token"
                    .parse::<TokenIssuer>()
                    .expect("issuer"),
                authority
                    .as_str()
                    .parse::<RegistryService>()
                    .expect("service"),
                SigningKey::hs256("test-v1".parse::<KeyId>().expect("key id"), &[7; 32])
                    .expect("signing key"),
                TokenLifetime::new(300).expect("lifetime"),
            ),
        })
    }

    fn sha256(bytes: &[u8]) -> String {
        format!("sha256:{:x}", Sha256::digest(bytes))
    }

    fn manifest_response(body: Vec<u8>) -> FakeResponse {
        FakeResponse {
            status: StatusCode::OK,
            digest: Some(sha256(&body)),
            body,
        }
    }

    #[tokio::test]
    async fn reads_and_validates_an_exact_subject_and_required_referrers() {
        let authority = RegistryAuthority::parse("registry.test").expect("authority");
        let namespace = RegistryNamespace::for_owner(RegistryOwner::PlatformBuilder {
            builder_key: PlatformBuilderKey::parse("rust-ubuntu").expect("key"),
        });
        let platform_digest = format!("sha256:{}", "b".repeat(64));
        let subject = format!(
            r#"{{"mediaType":"{OCI_INDEX_MEDIA_TYPE}","manifests":[{{"mediaType":"{OCI_MANIFEST_MEDIA_TYPE}","digest":"{platform_digest}","size":123,"platform":{{"os":"linux","architecture":"amd64"}}}}]}}"#
        )
        .into_bytes();
        let subject_digest = sha256(&subject);
        let mut routes = BTreeMap::new();
        let subject_path = format!("/v2/{}/manifests/{subject_digest}", namespace.as_str());
        routes.insert(subject_path, manifest_response(subject));

        let mut descriptors = Vec::new();
        for (kind, artifact) in [
            ("c", SBOM_ARTIFACT_TYPE),
            ("d", PROVENANCE_ARTIFACT_TYPE),
            ("e", SCAN_ARTIFACT_TYPE),
        ] {
            let blob_digest = format!("sha256:{}", kind.repeat(64));
            let body = format!(
                r#"{{"mediaType":"{OCI_MANIFEST_MEDIA_TYPE}","artifactType":"{artifact}","subject":{{"digest":"{subject_digest}"}},"blobs":[{{"mediaType":"application/octet-stream","digest":"{blob_digest}","size":1}}]}}"#
            )
            .into_bytes();
            let digest = sha256(&body);
            descriptors.push(format!(
                r#"{{"mediaType":"{OCI_MANIFEST_MEDIA_TYPE}","artifactType":"{artifact}","digest":"{digest}","size":{}}}"#,
                body.len()
            ));
            routes.insert(
                format!("/v2/{}/manifests/{digest}", namespace.as_str()),
                manifest_response(body),
            );
        }
        routes.insert(
            format!("/v2/{}/referrers/{subject_digest}", namespace.as_str()),
            FakeResponse {
                status: StatusCode::OK,
                digest: None,
                body: format!(r#"{{"manifests":[{}]}}"#, descriptors.join(",")).into_bytes(),
            },
        );
        let (origin, server) = serve(routes).await;
        let client = ZotHttpRegistry::new(
            ZotClientConfig::new(authority.clone(), &origin).expect("config"),
            tokens(&authority),
        )
        .expect("client");
        let reference = ImmutableManifestReference::new(
            authority,
            namespace,
            Sha256Digest::parse(subject_digest).expect("digest"),
        );
        let result = client.inspect_exact(&reference).await.expect("inspection");
        let ZotInspection::Present {
            platforms,
            evidence,
            ..
        } = result
        else {
            panic!("expected present graph");
        };
        assert_eq!(platforms.len(), 1);
        assert_eq!(evidence.referrers().len(), 3);
        server.abort();
    }

    #[tokio::test]
    async fn reports_an_exact_missing_digest_without_treating_it_as_an_outage() {
        let authority = RegistryAuthority::parse("registry.test").expect("authority");
        let namespace = RegistryNamespace::for_owner(RegistryOwner::PlatformBuilder {
            builder_key: PlatformBuilderKey::parse("python-ubuntu").expect("key"),
        });
        let (origin, server) = serve(BTreeMap::new()).await;
        let client = ZotHttpRegistry::new(
            ZotClientConfig::new(authority.clone(), &origin).expect("config"),
            tokens(&authority),
        )
        .expect("client");
        let reference = ImmutableManifestReference::new(
            authority,
            namespace,
            Sha256Digest::parse(format!("sha256:{}", "a".repeat(64))).expect("digest"),
        );
        assert_eq!(
            client.inspect_exact(&reference).await.expect("inspection"),
            ZotInspection::Missing
        );
        server.abort();
    }
}
