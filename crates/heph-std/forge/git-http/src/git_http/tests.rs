use super::{
    api::{AuthorizationRequest, GitAuthenticator, GitAuthorizer, GitOperation, Principal},
    auth::{OidcGitAuthenticator, PostgresGitAuthorizer, parse_basic_pat, pat_operation},
    backend::{BackendEnvironment, backend_command, parse_cgi_headers},
    errors::authentication_error_response,
    refs::diff_refs,
    service::{AuthenticatedHumanGitEndpoint, validate_backend_path},
};
use async_trait::async_trait;
use authz_domain::{
    AuthorizationDecision, AuthzError, GitRepositoryAuthorizer, GitRepositoryOperation,
};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use forge_domain::{CommitSha, GitRef, RepositoryId};
use identity_application::{
    ExternalIdentityVerificationError, ExternalIdentityVerifier, IdentityMappingError,
    VerifiedExternalIdentity, VerifiedIdentityMapper,
};
use identity_domain::{AuthenticatedIdentity, RequestId};
use pat_domain::{PersonalAccessToken, PersonalAccessTokenId};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    sync::Arc,
};
use uuid::Uuid;

struct UnexpectedRuntimeDelegate;

struct RejectingVerifier;

impl ExternalIdentityVerifier for RejectingVerifier {
    fn verify(
        &self,
        _token: &str,
        _expected_nonce: Option<&str>,
    ) -> Result<VerifiedExternalIdentity, ExternalIdentityVerificationError> {
        Err(ExternalIdentityVerificationError)
    }
}

struct UnexpectedIdentityMapper;

#[async_trait]
impl VerifiedIdentityMapper for UnexpectedIdentityMapper {
    async fn map_verified_identity(
        &self,
        _verified: &VerifiedExternalIdentity,
        _request_id: RequestId,
        _trace_id: Option<&str>,
    ) -> Result<AuthenticatedIdentity, IdentityMappingError> {
        panic!("identity mapping must not run after token denial")
    }
}

#[test]
fn authenticated_endpoint_closes_operation_query_and_receive_semantics() {
    assert_eq!(
        AuthenticatedHumanGitEndpoint::CloneInfoRefs.parameters(),
        (
            GitOperation::Clone,
            "info/refs",
            Some(String::from("service=git-upload-pack")),
            false
        )
    );
    assert_eq!(
        AuthenticatedHumanGitEndpoint::ReceivePack.parameters(),
        (GitOperation::Push, "git-receive-pack", None, true)
    );
}

#[async_trait]
impl GitRepositoryAuthorizer for UnexpectedRuntimeDelegate {
    async fn authorize_git(
        &self,
        _repository_id: Uuid,
        _operation: GitRepositoryOperation,
        _identity: &AuthenticatedIdentity,
    ) -> Result<AuthorizationDecision, AuthzError> {
        panic!("a runtime principal must not reach the human authorizer delegate")
    }
}

#[test]
fn runtime_principal_preserves_opaque_runtime_identity() {
    let principal = Principal::runtime("runtime-42", "session-opaque", "snapshot-opaque");

    assert_eq!(principal.name(), "runtime-42");
    assert!(principal.human_identity().is_none());
    let Principal::Runtime(runtime) = principal else {
        panic!("expected runtime principal");
    };
    assert_eq!(runtime.runtime_session_id(), "session-opaque");
    assert_eq!(runtime.authorization_snapshot_id(), "snapshot-opaque");
}

#[test]
fn parses_canonical_git_basic_pat_without_exposing_it() {
    let token = PersonalAccessToken::from_secret(PersonalAccessTokenId::new(), [0x5a; 32]);
    let plaintext = token.expose();
    let credential = format!(
        "Basic {}",
        BASE64_STANDARD.encode(format!("heph-pat:{plaintext}"))
    );

    let parsed = parse_basic_pat(&credential).expect("canonical Basic PAT");
    assert_eq!(parsed.id(), token.id());

    let malformed = format!("Basic {}", BASE64_STANDARD.encode("heph-pat:not-a-pat"));
    let error = parse_basic_pat(&malformed).expect_err("malformed PAT must fail closed");
    assert!(!error.to_string().contains("not-a-pat"));
    assert!(!format!("{error:?}").contains("not-a-pat"));
}

#[test]
fn maps_transport_operations_to_exact_pat_scope_operations() {
    assert_eq!(
        pat_operation(GitOperation::Clone),
        git_capability_domain::GitOperation::Discover
    );
    assert_eq!(
        pat_operation(GitOperation::Fetch),
        git_capability_domain::GitOperation::Fetch
    );
    assert_eq!(
        pat_operation(GitOperation::Push),
        git_capability_domain::GitOperation::Receive
    );
}

#[test]
fn authentication_denial_challenges_git_without_echoing_credentials() {
    let response = authentication_error_response("the Git credential is invalid");
    assert_eq!(response.status(), http::StatusCode::UNAUTHORIZED);
    assert_eq!(
        response.headers()[http::header::WWW_AUTHENTICATE],
        r#"Basic realm="hephaestus-git""#
    );
}

#[tokio::test]
async fn oidc_authenticator_denies_missing_and_invalid_bearers_without_disclosure() {
    let authenticator = OidcGitAuthenticator::new(
        Arc::new(RejectingVerifier),
        Arc::new(UnexpectedIdentityMapper),
    );
    let missing = authenticator
        .authenticate(None, RequestId::new())
        .await
        .expect_err("missing bearer token");
    assert_eq!(
        missing.to_string(),
        "Git authentication failed: a bearer token is required"
    );

    let invalid = authenticator
        .authenticate(Some("Bearer provider-secret"), RequestId::new())
        .await
        .expect_err("invalid bearer token");
    assert_eq!(
        invalid.to_string(),
        "Git authentication failed: the bearer token is invalid"
    );
    assert!(!invalid.to_string().contains("provider-secret"));
    assert!(!format!("{invalid:?}").contains("provider-secret"));
}

#[tokio::test]
async fn postgres_authorizer_rejects_unresolved_runtime_principal_before_human_delegate() {
    let authorizer = PostgresGitAuthorizer::new(Arc::new(UnexpectedRuntimeDelegate));
    let request = AuthorizationRequest {
        repository_id: RepositoryId::new(),
        operation: GitOperation::Fetch,
        principal: Principal::runtime("runtime-42", "session-opaque", "snapshot-opaque"),
    };

    let error = authorizer
        .authorize(&request)
        .await
        .expect_err("unresolved runtime principal must fail closed");
    assert_eq!(
        error.to_string(),
        "Git operation is not authorized: runtime Git authority is unavailable"
    );
}

#[test]
fn parses_native_backend_headers() {
    let (status, headers) =
        parse_cgi_headers(b"Status: 403 Forbidden\r\nContent-Type: text/plain").expect("headers");
    assert_eq!(status.as_u16(), 403);
    assert_eq!(headers["content-type"], "text/plain");
}

#[test]
fn computes_created_updated_and_deleted_refs() {
    let main = GitRef::parse("refs/heads/main").expect("ref");
    let tag = GitRef::parse("refs/tags/v1").expect("ref");
    let old = CommitSha::parse("a".repeat(40)).expect("sha");
    let new = CommitSha::parse("b".repeat(40)).expect("sha");
    let before = BTreeMap::from([(main.clone(), old.clone()), (tag, old)]);
    let after = BTreeMap::from([(main, new)]);
    let updates = diff_refs(&before, &after);
    assert_eq!(updates.len(), 2);
    assert!(updates.iter().any(|update| update.new_commit.is_none()));
    assert!(updates.iter().any(|update| update.old_commit.is_some()));
}

#[test]
fn rejects_relative_backend_executable() {
    assert!(validate_backend_path(Path::new("git-http-backend")).is_err());
    assert!(validate_backend_path(Path::new("/usr/libexec/git-core/git-http-backend")).is_ok());
}

#[tokio::test]
async fn backend_command_contains_only_allowlisted_cgi_environment() {
    let repository_id = RepositoryId::new();
    let environment = BackendEnvironment {
        project_root: Path::new("/srv/hephaestus/repositories"),
        repository_id,
        endpoint: "git-receive-pack",
        method: "POST",
        query: "",
        remote_user: "subject",
        content_type: Some("application/x-git-receive-pack-request"),
        content_length: Some("42"),
        git_protocol: Some("version=2"),
        runtime_receive_hook_directory: None,
        runtime_receive_context_file: None,
        runtime_receive_repository: None,
        runtime_receive_request_bytes: None,
        hidden_refs: &[],
    };
    let output = backend_command(Path::new("/usr/bin/env"), &environment)
        .output()
        .await
        .expect("run environment inspection helper");
    assert!(output.status.success());
    let actual = String::from_utf8(output.stdout)
        .expect("UTF-8 environment")
        .lines()
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    let expected = BTreeSet::from([
        String::from("CONTENT_LENGTH=42"),
        String::from("CONTENT_TYPE=application/x-git-receive-pack-request"),
        String::from("GATEWAY_INTERFACE=CGI/1.1"),
        String::from("GIT_HTTP_EXPORT_ALL=1"),
        String::from("GIT_PROJECT_ROOT=/srv/hephaestus/repositories"),
        String::from("GIT_PROTOCOL=version=2"),
        format!("PATH_INFO=/{repository_id}.git/git-receive-pack"),
        String::from("QUERY_STRING="),
        String::from("REMOTE_USER=subject"),
        String::from("REQUEST_METHOD=POST"),
        String::from("SERVER_PROTOCOL=HTTP/1.1"),
    ]);
    assert_eq!(actual, expected);
    assert!(!actual.iter().any(|value| {
        value.starts_with("HTTP_AUTHORIZATION=")
            || value.starts_with("AUTHORIZATION=")
            || value.starts_with("HOME=")
            || value.starts_with("PATH=")
    }));
}
