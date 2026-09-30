use crate::{
    AuthenticationError, AuthorizationError, AuthorizationRequest, GitAuthenticator, GitAuthorizer,
    GitOperation, Principal,
};
use async_trait::async_trait;
use authz_domain::{AuthorizationDecision, GitRepositoryAuthorizer, GitRepositoryOperation};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use forge_domain::RepositoryId;
use identity_application::{ExternalIdentityVerifier, VerifiedIdentityMapper};
use identity_domain::{AuthenticatedIdentity, RequestId};
use pat_domain::PersonalAccessToken;
use pat_postgres::PostgresPersonalAccessTokenService;
use runtime_git_authority::{RuntimeGitCredential, RuntimeGitCredentialRepository};
use std::sync::Arc;
use zeroize::Zeroize;

/// OIDC bearer-token authenticator backed by an injected identity mapper.
pub struct OidcGitAuthenticator {
    verifier: Arc<dyn ExternalIdentityVerifier>,
    mapper: Arc<dyn VerifiedIdentityMapper>,
}

impl OidcGitAuthenticator {
    /// Creates an OIDC-backed Git authenticator.
    #[must_use]
    pub const fn new(
        verifier: Arc<dyn ExternalIdentityVerifier>,
        mapper: Arc<dyn VerifiedIdentityMapper>,
    ) -> Self {
        Self { verifier, mapper }
    }
}

#[async_trait]
impl GitAuthenticator for OidcGitAuthenticator {
    async fn authenticate(
        &self,
        credential: Option<&str>,
        request_id: RequestId,
    ) -> Result<Principal, AuthenticationError> {
        let token = credential
            .and_then(|value| value.strip_prefix("Bearer "))
            .ok_or_else(|| AuthenticationError::denied("a bearer token is required"))?;
        let verified = self
            .verifier
            .verify(token, None)
            .map_err(|_| AuthenticationError::denied("the bearer token is invalid"))?;
        let identity = self
            .mapper
            .map_verified_identity(&verified, request_id, None)
            .await
            .map_err(|_| AuthenticationError::denied("the bearer identity is unavailable"))?;
        Ok(Principal::human(identity))
    }
}

/// Exact-run Git authenticator backed by hash-only runtime credential
/// persistence and immutable Git scope resolution.
pub struct RuntimeGitHttpAuthenticator {
    authority: Arc<dyn RuntimeGitCredentialRepository>,
}

impl RuntimeGitHttpAuthenticator {
    /// Creates a runtime Git authenticator over an application boundary.
    #[must_use]
    pub fn new(authority: Arc<dyn RuntimeGitCredentialRepository>) -> Self {
        Self { authority }
    }
}

#[async_trait]
impl GitAuthenticator for RuntimeGitHttpAuthenticator {
    async fn authenticate(
        &self,
        _credential: Option<&str>,
        _request_id: RequestId,
    ) -> Result<Principal, AuthenticationError> {
        Err(AuthenticationError::denied(
            "a repository-bound runtime Git credential is required",
        ))
    }

    async fn authenticate_git(
        &self,
        credential: Option<&str>,
        _request_id: RequestId,
        repository_id: RepositoryId,
        operation: GitOperation,
    ) -> Result<Principal, AuthenticationError> {
        let credential = credential
            .ok_or_else(|| AuthenticationError::denied("the Git credential is invalid"))?;
        let credential = parse_basic_runtime_git(credential)?;
        let authority = self
            .authority
            .authenticate(
                credential.storage_hash(),
                git_capability_domain::RepositoryId::new(repository_id.as_uuid()),
                capability_operation(operation),
                time::OffsetDateTime::now_utc(),
            )
            .await
            .map_err(|_| AuthenticationError::denied("the Git credential is invalid"))?;
        Ok(Principal::runtime_with_git_authority(
            format!("runtime:{}", authority.runtime_session_id),
            authority,
        ))
    }
}

/// Git authenticator accepting either OIDC bearer tokens or scoped developer
/// PATs supplied by a credential helper through HTTP Basic authentication.
pub struct CompositeGitAuthenticator {
    oidc: Arc<dyn GitAuthenticator>,
    personal_access_tokens: Arc<PostgresPersonalAccessTokenService>,
    runtime_git: Option<Arc<dyn GitAuthenticator>>,
}

impl CompositeGitAuthenticator {
    /// Creates a composite authenticator over the existing OIDC and PAT
    /// verification boundaries.
    #[must_use]
    pub const fn new(
        oidc: Arc<dyn GitAuthenticator>,
        personal_access_tokens: Arc<PostgresPersonalAccessTokenService>,
    ) -> Self {
        Self {
            oidc,
            personal_access_tokens,
            runtime_git: None,
        }
    }

    /// Adds the separately discriminated exact-run Git authenticator.
    #[must_use]
    pub fn with_runtime_git(mut self, runtime_git: Arc<dyn GitAuthenticator>) -> Self {
        self.runtime_git = Some(runtime_git);
        self
    }
}

#[async_trait]
impl GitAuthenticator for CompositeGitAuthenticator {
    async fn authenticate(
        &self,
        credential: Option<&str>,
        request_id: RequestId,
    ) -> Result<Principal, AuthenticationError> {
        self.oidc.authenticate(credential, request_id).await
    }

    async fn authenticate_git(
        &self,
        credential: Option<&str>,
        request_id: RequestId,
        repository_id: RepositoryId,
        operation: GitOperation,
    ) -> Result<Principal, AuthenticationError> {
        let credential = credential
            .ok_or_else(|| AuthenticationError::denied("a Git credential is required"))?;
        if credential.starts_with("Bearer ") {
            return self.oidc.authenticate(Some(credential), request_id).await;
        }
        if basic_username(credential) == Some("heph-runtime") {
            let runtime_git = self
                .runtime_git
                .as_ref()
                .ok_or_else(|| AuthenticationError::denied("the Git credential is invalid"))?;
            return runtime_git
                .authenticate_git(Some(credential), request_id, repository_id, operation)
                .await;
        }
        let token = parse_basic_pat(credential)?;
        let authenticated = self
            .personal_access_tokens
            .authenticate(&token, pat_operation(operation), repository_id, request_id)
            .await
            .map_err(|_| AuthenticationError::denied("the Git credential is invalid"))?;
        let identity = AuthenticatedIdentity::new(
            authenticated.owner_user_id,
            "urn:hephaestus:credential:pat",
            format!("user:{}", authenticated.owner_user_id),
            serde_json::json!({}),
            request_id,
        );
        Ok(Principal::human(identity))
    }
}

fn basic_username(credential: &str) -> Option<&str> {
    const MAX_BASIC_CREDENTIAL_BYTES: usize = 1_024;

    let encoded = credential
        .strip_prefix("Basic ")
        .filter(|encoded| !encoded.is_empty() && encoded.len() <= MAX_BASIC_CREDENTIAL_BYTES)?;
    let mut decoded = BASE64_STANDARD.decode(encoded).ok()?;
    let username_length = decoded.iter().position(|byte| *byte == b':')?;
    let username = std::str::from_utf8(&decoded[..username_length]).ok()?;
    // The returned names are static discriminators, never borrowed bearer
    // material. Wipe the decoded Basic payload before returning.
    let result = match username {
        "heph-runtime" => Some("heph-runtime"),
        "heph-pat" => Some("heph-pat"),
        _ => None,
    };
    decoded.zeroize();
    result
}

fn parse_basic_runtime_git(credential: &str) -> Result<RuntimeGitCredential, AuthenticationError> {
    const MAX_BASIC_CREDENTIAL_BYTES: usize = 1_024;

    let encoded = credential
        .strip_prefix("Basic ")
        .filter(|encoded| !encoded.is_empty() && encoded.len() <= MAX_BASIC_CREDENTIAL_BYTES)
        .ok_or_else(|| AuthenticationError::denied("the Git credential is invalid"))?;
    let mut decoded = BASE64_STANDARD
        .decode(encoded)
        .map_err(|_| AuthenticationError::denied("the Git credential is invalid"))?;
    let parsed = (|| {
        let decoded = std::str::from_utf8(&decoded)
            .map_err(|_| AuthenticationError::denied("the Git credential is invalid"))?;
        let (username, password) = decoded
            .split_once(':')
            .filter(|(_, password)| !password.contains(':'))
            .ok_or_else(|| AuthenticationError::denied("the Git credential is invalid"))?;
        if username != "heph-runtime" {
            return Err(AuthenticationError::denied("the Git credential is invalid"));
        }
        RuntimeGitCredential::parse(password)
            .map_err(|_| AuthenticationError::denied("the Git credential is invalid"))
    })();
    decoded.zeroize();
    parsed
}

pub fn parse_basic_pat(credential: &str) -> Result<PersonalAccessToken, AuthenticationError> {
    const MAX_BASIC_CREDENTIAL_BYTES: usize = 1_024;
    const PAT_USERNAME: &str = "heph-pat";

    let encoded = credential
        .strip_prefix("Basic ")
        .filter(|encoded| !encoded.is_empty() && encoded.len() <= MAX_BASIC_CREDENTIAL_BYTES)
        .ok_or_else(|| AuthenticationError::denied("the Git credential is invalid"))?;
    let mut decoded = BASE64_STANDARD
        .decode(encoded)
        .map_err(|_| AuthenticationError::denied("the Git credential is invalid"))?;
    let parsed = (|| {
        let decoded = std::str::from_utf8(&decoded)
            .map_err(|_| AuthenticationError::denied("the Git credential is invalid"))?;
        let (username, password) = decoded
            .split_once(':')
            .filter(|(_, password)| !password.contains(':'))
            .ok_or_else(|| AuthenticationError::denied("the Git credential is invalid"))?;
        if username != PAT_USERNAME {
            return Err(AuthenticationError::denied("the Git credential is invalid"));
        }
        PersonalAccessToken::parse(password)
            .map_err(|_| AuthenticationError::denied("the Git credential is invalid"))
    })();
    decoded.zeroize();
    parsed
}

pub const fn pat_operation(operation: GitOperation) -> git_capability_domain::GitOperation {
    match operation {
        GitOperation::Clone => git_capability_domain::GitOperation::Discover,
        GitOperation::Fetch => git_capability_domain::GitOperation::Fetch,
        GitOperation::Push => git_capability_domain::GitOperation::Receive,
    }
}

pub const fn capability_operation(operation: GitOperation) -> git_capability_domain::GitOperation {
    pat_operation(operation)
}

/// Database-native Git authorizer backed by the generated Mélange dispatcher.
pub struct PostgresGitAuthorizer {
    delegate: Arc<dyn GitRepositoryAuthorizer>,
}

impl PostgresGitAuthorizer {
    /// Creates a `PostgreSQL` Git authorizer.
    #[must_use]
    pub fn new(delegate: Arc<dyn GitRepositoryAuthorizer>) -> Self {
        Self { delegate }
    }
}

#[async_trait]
impl GitAuthorizer for PostgresGitAuthorizer {
    async fn authorize(&self, request: &AuthorizationRequest) -> Result<(), AuthorizationError> {
        if let Principal::Runtime(runtime) = &request.principal {
            let authority = runtime.git_authority().ok_or_else(|| {
                AuthorizationError::denied("runtime Git authority is unavailable")
            })?;
            let now = time::OffsetDateTime::now_utc().unix_timestamp();
            if authority.scope.repository_id().as_uuid() != request.repository_id.as_uuid()
                || !authority.scope.is_active_at(now)
                || !authority
                    .scope
                    .operations()
                    .contains(&capability_operation(request.operation))
            {
                return Err(AuthorizationError::denied(
                    "runtime Git authority does not match the request",
                ));
            }
            return Ok(());
        }
        let identity = request
            .principal
            .human_identity()
            .ok_or_else(|| AuthorizationError::denied("repository identity is unavailable"))?;
        let operation = match request.operation {
            GitOperation::Clone | GitOperation::Fetch => GitRepositoryOperation::Read,
            GitOperation::Push => GitRepositoryOperation::Write,
        };
        let decision = self
            .delegate
            .authorize_git(request.repository_id.as_uuid(), operation, identity)
            .await
            .map_err(|_| AuthorizationError::denied("authorization is unavailable"))?;
        if decision == AuthorizationDecision::Deny {
            return Err(AuthorizationError::denied(
                "repository permission was denied",
            ));
        }
        Ok(())
    }
}
