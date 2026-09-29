//! Focused fake-registry tests for exact Zot graph validation.

use super::model::{
    OCI_INDEX_MEDIA_TYPE, OCI_MANIFEST_MEDIA_TYPE, PROVENANCE_ARTIFACT_TYPE, SBOM_ARTIFACT_TYPE,
    SCAN_ARTIFACT_TYPE, ZotClientError,
};
use super::*;
use async_trait::async_trait;
use axum::{
    Router,
    body::Body,
    extract::{Request, State},
    http::{HeaderValue, Response},
};
use registry_domain::{
    ImmutableManifestReference, RegistryAuthority, RegistryNamespace, Sha256Digest,
};
use registry_domain::{PlatformImageKey, RegistryOwner};
use registry_reconciler::ZotInspection;
use registry_token::IssuedToken;
use registry_token::{
    AuthorizationDecision, KeyId, RegistryService, RegistryTokenIssuer, RepositoryActions,
    RepositoryName, ScopeRequest, SigningKey, TokenIssuer, TokenLifetime, TokenSubject,
    UnixTimestamp,
};
use reqwest::{StatusCode, header};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::sync::Arc;
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

async fn inspect_graph_with_evidence_field(
    evidence_field: &str,
) -> Result<ZotInspection, ZotClientError> {
    let authority = RegistryAuthority::parse("registry.test").expect("authority");
    let namespace = RegistryNamespace::for_owner(RegistryOwner::PlatformImage {
        image_key: PlatformImageKey::parse("rust-ubuntu").expect("key"),
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
            r#"{{"mediaType":"{OCI_MANIFEST_MEDIA_TYPE}","artifactType":"{artifact}","subject":{{"digest":"{subject_digest}"}},"{evidence_field}":[{{"mediaType":"application/octet-stream","digest":"{blob_digest}","size":1}}]}}"#
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
    let result = client.inspect_exact(&reference).await;
    server.abort();
    result
}

#[tokio::test]
async fn reads_and_validates_an_exact_subject_and_required_referrers() {
    let result = inspect_graph_with_evidence_field("layers")
        .await
        .expect("inspection");
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
}

#[tokio::test]
async fn rejects_legacy_blobs_without_oci_layers() {
    assert!(matches!(
        inspect_graph_with_evidence_field("blobs").await,
        Err(ZotClientError::InvalidGraph)
    ));
}

#[tokio::test]
async fn reports_an_exact_missing_digest_without_treating_it_as_an_outage() {
    let authority = RegistryAuthority::parse("registry.test").expect("authority");
    let namespace = RegistryNamespace::for_owner(RegistryOwner::PlatformImage {
        image_key: PlatformImageKey::parse("python-ubuntu").expect("key"),
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
