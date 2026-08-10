//! Authorized, bounded, streaming Git smart-HTTP transport.

pub mod receive_hook;
pub mod receive_policy;

use async_trait::async_trait;
use authz_domain::{AuthorizationDecision, GitRepositoryAuthorizer, GitRepositoryOperation};
use axum::{
    Router,
    body::Body,
    extract::{Path as AxumPath, Query, State},
    http::{HeaderMap, HeaderName, HeaderValue, Request, Response, StatusCode},
    response::IntoResponse,
    routing::{get, post},
};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use bytes::{Bytes, BytesMut};
use forge_domain::{CommitSha, GitRef, ReceiveId, RefUpdate, RepositoryId};
use forge_postgres::PgForgeRepository;
use forge_service::{ForgeRepositoryError, GitStorage};
use futures_util::StreamExt;
use identity_application::VerifiedIdentityMapper;
use identity_domain::{AuthenticatedIdentity, RequestId};
use identity_oidc::OidcVerifier;
use pat_domain::PersonalAccessToken;
use pat_postgres::PostgresPersonalAccessTokenService;
use runtime_git_authority::{
    AuthenticatedRuntimeGitAuthority, RuntimeGitCredential, RuntimeGitCredentialRepository,
};
use std::{
    collections::{BTreeMap, HashMap},
    io,
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::{ChildStdout, Command},
    sync::{Mutex, OwnedMutexGuard, mpsc},
};
use tokio_stream::wrappers::ReceiverStream;
use zeroize::Zeroize;

const MAX_CGI_HEADERS: usize = 32 * 1024;

/// Permission checked for a Git transport operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum GitOperation {
    /// Repository discovery for clone.
    Clone,
    /// Object transfer after discovery.
    Fetch,
    /// Reference advertisement or object transfer for push.
    Push,
}

/// Authenticated principal returned by a Git credential authenticator.
///
/// Human and runtime identities remain distinct so a runtime credential can
/// never accidentally inherit authorization through a human identity path.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Principal {
    /// A human authenticated through OIDC or a future user credential.
    Human(HumanPrincipal),
    /// One exact runtime session authenticated by a future runtime credential.
    Runtime(RuntimePrincipal),
}

impl Principal {
    /// Creates a human principal from a verified internal identity.
    #[must_use]
    pub fn human(identity: AuthenticatedIdentity) -> Self {
        Self::Human(HumanPrincipal {
            name: identity.subject.clone(),
            identity,
        })
    }

    /// Creates an exact-runtime principal from opaque control-plane IDs.
    #[must_use]
    pub fn runtime(
        name: impl Into<String>,
        runtime_session_id: impl Into<String>,
        authorization_snapshot_id: impl Into<String>,
    ) -> Self {
        Self::Runtime(RuntimePrincipal {
            name: name.into(),
            runtime_session_id: runtime_session_id.into(),
            authorization_snapshot_id: authorization_snapshot_id.into(),
            receive_context: None,
            git_authority: None,
        })
    }

    /// Creates an exact-runtime principal carrying host-resolved authority for
    /// Git's quarantined pre-receive boundary.
    #[must_use]
    pub fn runtime_with_receive_context(
        name: impl Into<String>,
        receive_context: receive_policy::ResolvedRuntimeReceiveContext,
    ) -> Self {
        Self::Runtime(RuntimePrincipal {
            name: name.into(),
            runtime_session_id: receive_context.runtime_session_id().to_owned(),
            authorization_snapshot_id: receive_context.authorization_snapshot_id().to_owned(),
            receive_context: Some(receive_context),
            git_authority: None,
        })
    }

    /// Creates an exact runtime principal from host-authenticated Git
    /// authority.
    #[must_use]
    pub fn runtime_with_git_authority(
        name: impl Into<String>,
        authority: AuthenticatedRuntimeGitAuthority,
    ) -> Self {
        Self::Runtime(RuntimePrincipal {
            name: name.into(),
            runtime_session_id: authority.runtime_session_id.to_string(),
            authorization_snapshot_id: authority.authorization_snapshot_id.to_string(),
            receive_context: None,
            git_authority: Some(authority),
        })
    }

    /// Returns the stable provider-neutral principal name exposed to Git.
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::Human(principal) => &principal.name,
            Self::Runtime(principal) => &principal.name,
        }
    }

    /// Returns a verified human identity, if this is a human principal.
    #[must_use]
    pub const fn human_identity(&self) -> Option<&AuthenticatedIdentity> {
        match self {
            Self::Human(principal) => Some(&principal.identity),
            Self::Runtime(_) => None,
        }
    }
}

/// A verified human Git principal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HumanPrincipal {
    name: String,
    identity: AuthenticatedIdentity,
}

/// An exact runtime Git principal whose identifiers remain opaque to transport.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimePrincipal {
    name: String,
    runtime_session_id: String,
    authorization_snapshot_id: String,
    receive_context: Option<receive_policy::ResolvedRuntimeReceiveContext>,
    git_authority: Option<AuthenticatedRuntimeGitAuthority>,
}

impl RuntimePrincipal {
    /// Returns the opaque runtime-session identifier.
    #[must_use]
    pub fn runtime_session_id(&self) -> &str {
        &self.runtime_session_id
    }

    /// Returns the opaque immutable authorization-snapshot identifier.
    #[must_use]
    pub fn authorization_snapshot_id(&self) -> &str {
        &self.authorization_snapshot_id
    }

    /// Returns host-resolved receive authority when this runtime credential
    /// permits a quarantined receive.
    #[must_use]
    pub const fn receive_context(&self) -> Option<&receive_policy::ResolvedRuntimeReceiveContext> {
        self.receive_context.as_ref()
    }

    /// Returns complete host-resolved Git authority for this request.
    #[must_use]
    pub const fn git_authority(&self) -> Option<&AuthenticatedRuntimeGitAuthority> {
        self.git_authority.as_ref()
    }
}

/// Owned authorization input.
#[derive(Debug, Clone)]
pub struct AuthorizationRequest {
    /// Repository being accessed.
    pub repository_id: RepositoryId,
    /// Requested operation.
    pub operation: GitOperation,
    /// Principal established by the HTTP authentication middleware.
    pub principal: Principal,
}

/// Authentication boundary used by Git HTTP middleware.
#[async_trait]
pub trait GitAuthenticator: Send + Sync + 'static {
    /// Consumes an HTTP credential and returns a verified request principal.
    ///
    /// # Errors
    ///
    /// Returns a non-sensitive error when the credential cannot be verified or
    /// mapped to an active internal user.
    async fn authenticate(
        &self,
        credential: Option<&str>,
        request_id: RequestId,
    ) -> Result<Principal, AuthenticationError>;

    /// Consumes a Git HTTP credential bound to its exact repository operation.
    ///
    /// Authenticators without token-local Git scope delegate to
    /// [`Self::authenticate`]. Scoped credentials override this method so the
    /// credential cannot be accepted before its repository and operation are
    /// known.
    ///
    /// # Errors
    ///
    /// Returns a non-sensitive error when the credential is invalid for the
    /// exact repository operation.
    async fn authenticate_git(
        &self,
        credential: Option<&str>,
        request_id: RequestId,
        repository_id: RepositoryId,
        operation: GitOperation,
    ) -> Result<Principal, AuthenticationError> {
        let _ = (repository_id, operation);
        self.authenticate(credential, request_id).await
    }
}

/// Authorization boundary for every Git operation.
#[async_trait]
pub trait GitAuthorizer: Send + Sync + 'static {
    /// Authorizes one operation and returns its authenticated principal.
    ///
    /// # Errors
    ///
    /// Returns an error without invoking Git when access is denied or identity
    /// resolution fails.
    async fn authorize(&self, request: &AuthorizationRequest) -> Result<(), AuthorizationError>;
}

/// OIDC bearer-token authenticator backed by an injected identity mapper.
pub struct OidcGitAuthenticator {
    verifier: Arc<OidcVerifier>,
    mapper: Arc<dyn VerifiedIdentityMapper>,
}

impl OidcGitAuthenticator {
    /// Creates an OIDC-backed Git authenticator.
    #[must_use]
    pub const fn new(verifier: Arc<OidcVerifier>, mapper: Arc<dyn VerifiedIdentityMapper>) -> Self {
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

fn parse_basic_pat(credential: &str) -> Result<PersonalAccessToken, AuthenticationError> {
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

const fn pat_operation(operation: GitOperation) -> git_capability_domain::GitOperation {
    match operation {
        GitOperation::Clone => git_capability_domain::GitOperation::Discover,
        GitOperation::Fetch => git_capability_domain::GitOperation::Fetch,
        GitOperation::Push => git_capability_domain::GitOperation::Receive,
    }
}

const fn capability_operation(operation: GitOperation) -> git_capability_domain::GitOperation {
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

/// Authentication failure.
#[derive(Debug, Clone, thiserror::Error)]
#[error("Git authentication failed: {message}")]
pub struct AuthenticationError {
    message: String,
}

impl AuthenticationError {
    /// Creates an authentication failure with a non-sensitive explanation.
    #[must_use]
    pub fn denied(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

/// Authorization failure.
#[derive(Debug, Clone, thiserror::Error)]
#[error("Git operation is not authorized: {message}")]
pub struct AuthorizationError {
    message: String,
}

impl AuthorizationError {
    /// Creates a denial with a non-sensitive explanation.
    #[must_use]
    pub fn denied(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

/// Resource limits for one native backend transaction.
#[derive(Debug, Clone)]
pub struct GitHttpLimits {
    /// Maximum request bytes streamed into Git.
    pub max_request_bytes: u64,
    /// Maximum response bytes streamed out of Git.
    pub max_response_bytes: u64,
    /// Maximum wall-clock transaction duration.
    pub transaction_timeout: Duration,
}

impl Default for GitHttpLimits {
    fn default() -> Self {
        Self {
            max_request_bytes: 2 * 1024 * 1024 * 1024,
            max_response_bytes: 2 * 1024 * 1024 * 1024,
            transaction_timeout: Duration::from_secs(15 * 60),
        }
    }
}

/// Configured Git smart-HTTP service.
#[derive(Clone)]
pub struct GitHttpService {
    repository: Arc<PgForgeRepository>,
    storage: Arc<GitStorage>,
    authenticator: Arc<dyn GitAuthenticator>,
    authorizer: Arc<dyn GitAuthorizer>,
    backend: PathBuf,
    limits: GitHttpLimits,
    receive_locks: Arc<Mutex<HashMap<RepositoryId, Arc<Mutex<()>>>>>,
    runtime_receive_hook: Option<PathBuf>,
}

impl GitHttpService {
    /// Creates a service around a resolved `git-http-backend` executable.
    ///
    /// # Errors
    ///
    /// Returns an error unless the configured executable path is absolute.
    pub fn new(
        repository: Arc<PgForgeRepository>,
        storage: Arc<GitStorage>,
        authenticator: Arc<dyn GitAuthenticator>,
        authorizer: Arc<dyn GitAuthorizer>,
        backend: PathBuf,
        limits: GitHttpLimits,
    ) -> Result<Self, GitHttpError> {
        validate_backend_path(&backend)?;
        Ok(Self {
            repository,
            storage,
            authenticator,
            authorizer,
            backend,
            limits,
            receive_locks: Arc::new(Mutex::new(HashMap::new())),
            runtime_receive_hook: None,
        })
    }

    /// Installs the absolute host-owned `pre-receive` executable used only for
    /// runtime receives. Human pushes retain the repository's existing hook
    /// configuration.
    ///
    /// # Errors
    ///
    /// Returns an error unless the path is absolute and names `pre-receive`.
    pub fn with_runtime_receive_hook(mut self, hook: PathBuf) -> Result<Self, GitHttpError> {
        if !hook.is_absolute() || hook.file_name() != Some(std::ffi::OsStr::new("pre-receive")) {
            return Err(GitHttpError::InvalidReceiveHookPath(hook));
        }
        self.runtime_receive_hook = Some(hook);
        Ok(self)
    }

    /// Builds Axum routes rooted at `/{repository_id}`.
    pub fn router(self) -> Router {
        let service = Arc::new(self);
        Router::new()
            .route("/{repository}/info/refs", get(info_refs))
            .route("/{repository}/git-upload-pack", post(upload_pack))
            .route("/{repository}/git-receive-pack", post(receive_pack))
            .with_state(Arc::clone(&service))
    }

    async fn lock_receive(&self, repository_id: RepositoryId) -> OwnedMutexGuard<()> {
        let lock = self
            .receive_locks
            .lock()
            .await
            .entry(repository_id)
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone();
        lock.lock_owned().await
    }
}

fn validate_backend_path(backend: &Path) -> Result<(), GitHttpError> {
    if backend.is_absolute() {
        Ok(())
    } else {
        Err(GitHttpError::InvalidBackendPath(backend.to_owned()))
    }
}

#[derive(Debug, serde::Deserialize)]
struct InfoRefsQuery {
    service: String,
}

async fn info_refs(
    State(service): State<Arc<GitHttpService>>,
    AxumPath(repository): AxumPath<String>,
    Query(query): Query<InfoRefsQuery>,
    request: Request<Body>,
) -> Response<Body> {
    let (operation, endpoint) = match query.service.as_str() {
        "git-upload-pack" => (GitOperation::Clone, "info/refs"),
        "git-receive-pack" => (GitOperation::Push, "info/refs"),
        _ => return error_response(StatusCode::BAD_REQUEST, "unsupported Git service"),
    };
    execute(
        service,
        repository,
        operation,
        endpoint,
        Some(format!("service={}", query.service)),
        request,
        false,
    )
    .await
}

async fn upload_pack(
    State(service): State<Arc<GitHttpService>>,
    AxumPath(repository): AxumPath<String>,
    request: Request<Body>,
) -> Response<Body> {
    execute(
        service,
        repository,
        GitOperation::Fetch,
        "git-upload-pack",
        None,
        request,
        false,
    )
    .await
}

async fn receive_pack(
    State(service): State<Arc<GitHttpService>>,
    AxumPath(repository): AxumPath<String>,
    request: Request<Body>,
) -> Response<Body> {
    execute(
        service,
        repository,
        GitOperation::Push,
        "git-receive-pack",
        None,
        request,
        true,
    )
    .await
}

#[allow(clippy::too_many_lines)]
async fn execute(
    service: Arc<GitHttpService>,
    route: String,
    operation: GitOperation,
    endpoint: &'static str,
    query: Option<String>,
    mut request: Request<Body>,
    receive: bool,
) -> Response<Body> {
    let repository_id = match GitStorage::parse_route(&route) {
        Ok(id) => id,
        Err(error) => return error_response(StatusCode::BAD_REQUEST, &error.to_string()),
    };
    let mut credential = request
        .headers_mut()
        .remove(http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok().map(str::to_owned));
    let principal = service
        .authenticator
        .authenticate_git(
            credential.as_deref(),
            RequestId::new(),
            repository_id,
            operation,
        )
        .await;
    if let Some(credential) = &mut credential {
        credential.zeroize();
    }
    let principal = match principal {
        Ok(principal) => principal,
        Err(error) => return authentication_error_response(&error.to_string()),
    };
    match service
        .authorizer
        .authorize(&AuthorizationRequest {
            repository_id,
            operation,
            principal: principal.clone(),
        })
        .await
    {
        Ok(()) => {}
        Err(error) => return error_response(StatusCode::FORBIDDEN, &error.to_string()),
    }
    let principal_identity = principal.human_identity().cloned();
    let runtime_receive_context = if receive {
        match &principal {
            Principal::Human(_) => None,
            Principal::Runtime(runtime) => {
                let context = runtime.git_authority().map_or_else(
                    || runtime.receive_context().cloned(),
                    |authority| {
                        receive_policy::ResolvedRuntimeReceiveContext::new_with_expected_parent(
                            Arc::clone(&authority.scope),
                            authority.runtime_session_id.to_string(),
                            authority.authorization_snapshot_id.to_string(),
                            authority.evaluated_at.unix_timestamp(),
                            authority.expected_parent.clone(),
                        )
                        .ok()
                    },
                );
                let Some(context) = context else {
                    return error_response(
                        StatusCode::FORBIDDEN,
                        "runtime receive authority is unavailable",
                    );
                };
                let now = match SystemTime::now().duration_since(UNIX_EPOCH) {
                    Ok(duration) => i64::try_from(duration.as_secs()).unwrap_or(i64::MAX),
                    Err(_) => {
                        return error_response(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "host clock is unavailable",
                        );
                    }
                };
                if context.repository_id()
                    != git_capability_domain::RepositoryId::new(repository_id.as_uuid())
                    || !context.is_active_at(now)
                {
                    return error_response(
                        StatusCode::FORBIDDEN,
                        "runtime receive authority is invalid",
                    );
                }
                if service.runtime_receive_hook.is_none() {
                    return error_response(
                        StatusCode::FORBIDDEN,
                        "runtime receive policy guard is unavailable",
                    );
                }
                Some(context)
            }
        }
    } else {
        None
    };
    let repository_result = match &principal_identity {
        Some(identity) => {
            service
                .repository
                .get_repository_as(repository_id, identity)
                .await
        }
        None => service.repository.get_repository(repository_id).await,
    };
    let repository = match repository_result {
        Ok(repository) => repository,
        Err(ForgeRepositoryError::RepositoryNotFound(_)) => {
            return error_response(StatusCode::NOT_FOUND, "repository was not found");
        }
        Err(error) => return service_error(error),
    };
    let runtime_scope = match &principal {
        Principal::Human(_) => None,
        Principal::Runtime(runtime) => runtime
            .git_authority()
            .map(|authority| Arc::clone(&authority.scope)),
    };
    // Runtime ref visibility is computed from canonical refs. Serialize it
    // with receives so a concurrent human push cannot introduce an
    // out-of-scope advertisement after filtering but before upload-pack.
    let receive_guard = if receive || runtime_scope.is_some() {
        Some(service.lock_receive(repository_id).await)
    } else {
        None
    };
    let hidden_refs = if let Some(scope) = runtime_scope.as_deref() {
        match hidden_runtime_refs(
            service.storage.repository_path(repository_id),
            scope,
            capability_operation(operation),
        )
        .await
        {
            Ok(refs) => refs,
            Err(error) => return service_error(error),
        }
    } else {
        Vec::new()
    };
    let declared_content_length = request
        .headers()
        .get(http::header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok());
    let max_request_bytes =
        runtime_receive_context
            .as_ref()
            .map_or(service.limits.max_request_bytes, |context| {
                service
                    .limits
                    .max_request_bytes
                    .min(context.transfer_limits().request_bytes())
            });
    if let Some(content_length) = declared_content_length {
        if content_length > max_request_bytes {
            return error_response(StatusCode::PAYLOAD_TOO_LARGE, "Git request exceeds limit");
        }
    }

    let receive_id = receive.then(ReceiveId::new);
    let before = if receive {
        match snapshot_refs(service.storage.repository_path(repository_id)).await {
            Ok(refs) => Some(refs),
            Err(error) => return service_error(error),
        }
    } else {
        None
    };
    let hook_context = match runtime_receive_context
        .as_ref()
        .map(materialize_receive_context)
        .transpose()
    {
        Ok(context) => context,
        Err(error) => return service_error(error),
    };
    let runtime_hook_directory = runtime_receive_context.as_ref().and_then(|_| {
        service
            .runtime_receive_hook
            .as_deref()
            .and_then(Path::parent)
    });
    let request_bytes_bound = runtime_receive_context.as_ref().map(|_| {
        declared_content_length
            .unwrap_or(max_request_bytes)
            .to_string()
    });
    let repository_id_text = repository_id.to_string();
    let environment = BackendEnvironment {
        project_root: service.storage.root(),
        repository_id,
        endpoint,
        method: request.method().as_str(),
        query: query.as_deref().unwrap_or(""),
        remote_user: principal.name(),
        content_type: header_value(&request, http::header::CONTENT_TYPE),
        content_length: header_value(&request, http::header::CONTENT_LENGTH),
        git_protocol: header_value_name(&request, "git-protocol"),
        runtime_receive_hook_directory: runtime_hook_directory,
        runtime_receive_context_file: hook_context.as_ref().map(tempfile::NamedTempFile::path),
        runtime_receive_repository: runtime_receive_context
            .as_ref()
            .map(|_| repository_id_text.as_str()),
        runtime_receive_request_bytes: request_bytes_bound.as_deref(),
        hidden_refs: &hidden_refs,
    };
    let mut command = backend_command(&service.backend, &environment);

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => return service_error(error),
    };
    let Some(mut stdin) = child.stdin.take() else {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Git backend has no stdin",
        );
    };
    let Some(mut stdout) = child.stdout.take() else {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Git backend has no stdout",
        );
    };
    let Some(stderr) = child.stderr.take() else {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Git backend has no stderr",
        );
    };

    let mut body_stream = request.into_body().into_data_stream();
    let request_writer = tokio::spawn(async move {
        let mut written = 0_u64;
        while let Some(chunk) = body_stream.next().await {
            let chunk = chunk.map_err(io::Error::other)?;
            written = written
                .checked_add(u64::try_from(chunk.len()).unwrap_or(u64::MAX))
                .ok_or_else(|| io::Error::other("Git request length overflow"))?;
            if written > max_request_bytes {
                return Err(io::Error::other("Git request exceeds configured limit"));
            }
            stdin.write_all(&chunk).await?;
        }
        stdin.shutdown().await
    });
    let stderr_reader = tokio::spawn(async move {
        let mut bytes = Vec::new();
        stderr.take(64 * 1024).read_to_end(&mut bytes).await?;
        Ok::<_, io::Error>(bytes)
    });

    let (status, response_headers, remainder) = match read_cgi_headers(&mut stdout).await {
        Ok(headers) => headers,
        Err(error) => return service_error(error),
    };
    let (sender, receiver) = mpsc::channel::<Result<Bytes, io::Error>>(8);
    // Keep the HTTP body open until backend completion and receive durability
    // are known, so a push cannot report transport completion first.
    let completion_sender = sender.clone();
    let max_response_bytes = service.limits.max_response_bytes;
    let response_reader = tokio::spawn(stream_response(
        stdout,
        sender,
        remainder,
        max_response_bytes,
    ));

    let repository_service = Arc::clone(&service.repository);
    let repository_for_receive = repository.clone();
    let repository_path = service.storage.repository_path(repository_id);
    let principal_name = principal.name().to_owned();
    let timeout = service.limits.transaction_timeout;
    tokio::spawn(async move {
        // Keep the owner-only authority file alive only while this exact
        // backend transaction and its hook descendants can use the handle.
        let runtime_receive_context_file = hook_context;
        let receive_guard = receive_guard;
        let completion = tokio::time::timeout(timeout, child.wait()).await;
        let status = match completion {
            Ok(Ok(status)) => status,
            Ok(Err(error)) => {
                send_stream_error(&completion_sender, error.to_string()).await;
                tracing::warn!(%repository_id, ?receive_id, %error, "Git backend wait failed");
                return;
            }
            Err(_) => {
                if let Err(error) = child.kill().await {
                    tracing::warn!(%repository_id, ?receive_id, %error, "timed-out Git backend could not be killed");
                }
                send_stream_error(&completion_sender, "Git transaction timed out").await;
                tracing::warn!(%repository_id, ?receive_id, "Git backend transaction timed out");
                return;
            }
        };
        let writer_result = request_writer.await;
        let reader_result = response_reader.await;
        let stderr = stderr_reader
            .await
            .ok()
            .and_then(Result::ok)
            .unwrap_or_default();
        if !status.success()
            || !matches!(writer_result, Ok(Ok(())))
            || !matches!(reader_result, Ok(Ok(())))
        {
            send_stream_error(&completion_sender, "Git backend transaction failed").await;
            tracing::warn!(
                %repository_id,
                ?receive_id,
                backend_status = ?status.code(),
                stderr = %String::from_utf8_lossy(&stderr),
                "Git backend transaction failed"
            );
            return;
        }
        if let (Some(receive_id), Some(before)) = (receive_id, before) {
            match snapshot_refs(repository_path).await {
                Ok(after) => {
                    let updates = diff_refs(&before, &after);
                    if updates.is_empty() {
                        tracing::debug!(%repository_id, %receive_id, "push changed no refs");
                        return;
                    }
                    if let Err(error) = repository_service
                        .accept_receive_as(
                            &repository_for_receive,
                            receive_id,
                            &principal_name,
                            principal_identity.as_ref(),
                            &updates,
                        )
                        .await
                    {
                        send_stream_error(
                            &completion_sender,
                            format!("accepted receive persistence failed: {error}"),
                        )
                        .await;
                        tracing::error!(
                            %repository_id,
                            %receive_id,
                            %error,
                            "accepted Git receive could not be persisted"
                        );
                    } else {
                        tracing::info!(
                            %repository_id,
                            %receive_id,
                            ref_updates = updates.len(),
                            "accepted Git receive was persisted"
                        );
                    }
                }
                Err(error) => {
                    send_stream_error(
                        &completion_sender,
                        format!("accepted receive inspection failed: {error}"),
                    )
                    .await;
                    tracing::error!(
                        %repository_id,
                        %receive_id,
                        %error,
                        "accepted Git receive refs could not be inspected"
                    );
                }
            }
        }
        drop(receive_guard);
        drop(runtime_receive_context_file);
    });

    let mut builder = Response::builder().status(status);
    for (name, value) in &response_headers {
        builder = builder.header(name, value);
    }
    builder
        .body(Body::from_stream(ReceiverStream::new(receiver)))
        .unwrap_or_else(service_error)
}

async fn send_stream_error(
    sender: &mpsc::Sender<Result<Bytes, io::Error>>,
    message: impl Into<String>,
) {
    let _ = sender.send(Err(io::Error::other(message.into()))).await;
}

fn header_value(request: &Request<Body>, header: HeaderName) -> Option<&str> {
    request
        .headers()
        .get(header)
        .and_then(|value| value.to_str().ok())
}

fn header_value_name<'request>(
    request: &'request Request<Body>,
    header: &str,
) -> Option<&'request str> {
    request
        .headers()
        .get(header)
        .and_then(|value| value.to_str().ok())
}

struct BackendEnvironment<'a> {
    project_root: &'a Path,
    repository_id: RepositoryId,
    endpoint: &'a str,
    method: &'a str,
    query: &'a str,
    remote_user: &'a str,
    content_type: Option<&'a str>,
    content_length: Option<&'a str>,
    git_protocol: Option<&'a str>,
    runtime_receive_hook_directory: Option<&'a Path>,
    runtime_receive_context_file: Option<&'a Path>,
    runtime_receive_repository: Option<&'a str>,
    runtime_receive_request_bytes: Option<&'a str>,
    hidden_refs: &'a [String],
}

fn backend_command(backend: &Path, environment: &BackendEnvironment<'_>) -> Command {
    let mut command = Command::new(backend);
    command
        .env_clear()
        .env("GIT_PROJECT_ROOT", environment.project_root)
        .env("GIT_HTTP_EXPORT_ALL", "1")
        .env(
            "PATH_INFO",
            format!(
                "/{}.git/{}",
                environment.repository_id, environment.endpoint
            ),
        )
        .env("REQUEST_METHOD", environment.method)
        .env("QUERY_STRING", environment.query)
        .env("GATEWAY_INTERFACE", "CGI/1.1")
        .env("SERVER_PROTOCOL", "HTTP/1.1")
        .env("REMOTE_USER", environment.remote_user)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    for (name, value) in [
        ("CONTENT_TYPE", environment.content_type),
        ("CONTENT_LENGTH", environment.content_length),
        ("GIT_PROTOCOL", environment.git_protocol),
    ] {
        if let Some(value) = value {
            command.env(name, value);
        }
    }
    let mut configuration = Vec::<(&str, String)>::new();
    if let Some(hook_directory) = environment.runtime_receive_hook_directory {
        configuration.push((
            "core.hooksPath",
            hook_directory.to_string_lossy().into_owned(),
        ));
    }
    for reference in environment.hidden_refs {
        configuration.push(("uploadpack.hideRefs", reference.clone()));
        configuration.push(("receive.hideRefs", reference.clone()));
    }
    if !configuration.is_empty() {
        command.env("GIT_CONFIG_COUNT", configuration.len().to_string());
        for (index, (key, value)) in configuration.iter().enumerate() {
            command
                .env(format!("GIT_CONFIG_KEY_{index}"), key)
                .env(format!("GIT_CONFIG_VALUE_{index}"), value);
        }
    }
    for (name, value) in [
        (
            "HEPH_RUNTIME_RECEIVE_CONTEXT_FILE",
            environment
                .runtime_receive_context_file
                .map(Path::as_os_str)
                .and_then(std::ffi::OsStr::to_str),
        ),
        (
            "HEPH_RUNTIME_RECEIVE_REPOSITORY",
            environment.runtime_receive_repository,
        ),
        (
            "HEPH_RUNTIME_RECEIVE_REQUEST_BYTES",
            environment.runtime_receive_request_bytes,
        ),
    ] {
        if let Some(value) = value {
            command.env(name, value);
        }
    }
    command
}

fn materialize_receive_context(
    context: &receive_policy::ResolvedRuntimeReceiveContext,
) -> Result<tempfile::NamedTempFile, GitHttpError> {
    let bytes = context.to_hook_json().map_err(domain)?;
    let mut file = tempfile::Builder::new()
        .prefix("hephaestus-runtime-git-")
        .suffix(".context")
        .tempfile()
        .map_err(GitHttpError::Io)?;
    std::io::Write::write_all(file.as_file_mut(), &bytes).map_err(GitHttpError::Io)?;
    file.as_file().sync_all().map_err(GitHttpError::Io)?;
    Ok(file)
}

async fn hidden_runtime_refs(
    repository: PathBuf,
    scope: &git_capability_domain::GitCapabilityScope,
    operation: git_capability_domain::GitOperation,
) -> Result<Vec<String>, GitHttpError> {
    Ok(snapshot_refs(repository)
        .await?
        .into_keys()
        .filter(|reference| !scope.allows(operation, reference.as_str()))
        .map(|reference| reference.as_str().to_owned())
        .collect())
}

async fn read_cgi_headers(
    stdout: &mut ChildStdout,
) -> Result<(StatusCode, HeaderMap, Bytes), GitHttpError> {
    let mut buffer = BytesMut::with_capacity(1024);
    loop {
        if let Some((end, delimiter_length)) = find_header_end(&buffer) {
            let raw = buffer.split_to(end);
            let _delimiter = buffer.split_to(delimiter_length);
            let (status, headers) = parse_cgi_headers(&raw)?;
            return Ok((status, headers, buffer.freeze()));
        }
        if buffer.len() >= MAX_CGI_HEADERS {
            return Err(GitHttpError::InvalidBackendResponse(
                "CGI headers exceed configured limit",
            ));
        }
        let read = stdout
            .read_buf(&mut buffer)
            .await
            .map_err(GitHttpError::Io)?;
        if read == 0 {
            return Err(GitHttpError::InvalidBackendResponse(
                "CGI response ended before its headers",
            ));
        }
    }
}

fn find_header_end(bytes: &[u8]) -> Option<(usize, usize)> {
    bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|position| (position, 4))
        .or_else(|| {
            bytes
                .windows(2)
                .position(|window| window == b"\n\n")
                .map(|position| (position, 2))
        })
}

fn parse_cgi_headers(bytes: &[u8]) -> Result<(StatusCode, HeaderMap), GitHttpError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| GitHttpError::InvalidBackendResponse("CGI headers are not UTF-8"))?;
    let mut status = StatusCode::OK;
    let mut headers = HeaderMap::new();
    for line in text.lines() {
        let (name, value) = line
            .split_once(':')
            .ok_or(GitHttpError::InvalidBackendResponse("malformed CGI header"))?;
        if name.eq_ignore_ascii_case("status") {
            let code = value
                .trim()
                .split_once(' ')
                .map_or_else(|| value.trim(), |(code, _)| code);
            status = StatusCode::from_bytes(code.as_bytes())
                .map_err(|_| GitHttpError::InvalidBackendResponse("invalid CGI status"))?;
            continue;
        }
        let name = HeaderName::from_bytes(name.trim().as_bytes())
            .map_err(|_| GitHttpError::InvalidBackendResponse("invalid CGI header name"))?;
        let value = HeaderValue::from_str(value.trim())
            .map_err(|_| GitHttpError::InvalidBackendResponse("invalid CGI header value"))?;
        headers.append(name, value);
    }
    Ok((status, headers))
}

async fn stream_response(
    mut stdout: ChildStdout,
    sender: mpsc::Sender<Result<Bytes, io::Error>>,
    remainder: Bytes,
    maximum: u64,
) -> io::Result<()> {
    let mut total = u64::try_from(remainder.len()).unwrap_or(u64::MAX);
    if total > maximum {
        let _ = sender
            .send(Err(io::Error::other(
                "Git response exceeds configured limit",
            )))
            .await;
        return Err(io::Error::other("Git response exceeds configured limit"));
    }
    if !remainder.is_empty() && sender.send(Ok(remainder)).await.is_err() {
        return Ok(());
    }
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let read = stdout.read(&mut buffer).await?;
        if read == 0 {
            return Ok(());
        }
        total = total
            .checked_add(u64::try_from(read).unwrap_or(u64::MAX))
            .ok_or_else(|| io::Error::other("Git response length overflow"))?;
        if total > maximum {
            let _ = sender
                .send(Err(io::Error::other(
                    "Git response exceeds configured limit",
                )))
                .await;
            return Err(io::Error::other("Git response exceeds configured limit"));
        }
        if sender
            .send(Ok(Bytes::copy_from_slice(&buffer[..read])))
            .await
            .is_err()
        {
            return Ok(());
        }
    }
}

async fn snapshot_refs(path: PathBuf) -> Result<BTreeMap<GitRef, CommitSha>, GitHttpError> {
    let output = Command::new("git")
        .arg("--git-dir")
        .arg(path)
        .arg("for-each-ref")
        .arg("--format=%(refname) %(objectname)")
        .env_clear()
        .stdin(Stdio::null())
        .output()
        .await
        .map_err(GitHttpError::Io)?;
    if !output.status.success() {
        return Err(GitHttpError::Git(
            String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        ));
    }
    let text = std::str::from_utf8(&output.stdout)
        .map_err(|_| GitHttpError::Git(String::from("ref output was not UTF-8")))?;
    text.lines()
        .map(|line| {
            let (name, commit) = line
                .split_once(' ')
                .ok_or_else(|| GitHttpError::Git(String::from("malformed ref output")))?;
            Ok((
                GitRef::parse(name.to_owned()).map_err(domain)?,
                CommitSha::parse(commit.to_owned()).map_err(domain)?,
            ))
        })
        .collect()
}

fn diff_refs(
    before: &BTreeMap<GitRef, CommitSha>,
    after: &BTreeMap<GitRef, CommitSha>,
) -> Vec<RefUpdate> {
    let mut names = before
        .keys()
        .chain(after.keys())
        .cloned()
        .collect::<Vec<_>>();
    names.sort();
    names.dedup();
    names
        .into_iter()
        .filter_map(|git_ref| {
            let old_commit = before.get(&git_ref).cloned();
            let new_commit = after.get(&git_ref).cloned();
            (old_commit != new_commit).then_some(RefUpdate {
                git_ref,
                old_commit,
                new_commit,
            })
        })
        .collect()
}

fn domain(error: impl std::fmt::Display) -> GitHttpError {
    GitHttpError::Git(error.to_string())
}

fn error_response(status: StatusCode, message: &str) -> Response<Body> {
    (
        status,
        [(http::header::CONTENT_TYPE, "application/json")],
        serde_json::to_string(&serde_json::json!({"error": message}))
            .unwrap_or_else(|_| String::from(r#"{"error":"internal error"}"#)),
    )
        .into_response()
}

fn authentication_error_response(message: &str) -> Response<Body> {
    let mut response = error_response(StatusCode::UNAUTHORIZED, message);
    response.headers_mut().insert(
        http::header::WWW_AUTHENTICATE,
        HeaderValue::from_static(r#"Basic realm="hephaestus-git""#),
    );
    response
}

fn service_error(error: impl std::fmt::Display) -> Response<Body> {
    tracing::warn!(%error, "Git HTTP request failed");
    error_response(StatusCode::INTERNAL_SERVER_ERROR, &error.to_string())
}

/// Native Git HTTP adapter failure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum GitHttpError {
    /// Configured backend executable is not an absolute path.
    #[error("git-http-backend path must be absolute: {0}")]
    InvalidBackendPath(PathBuf),
    /// Configured runtime pre-receive hook path is not absolute or canonical.
    #[error("runtime pre-receive hook must be an absolute path named pre-receive: {0}")]
    InvalidReceiveHookPath(PathBuf),
    /// Process I/O failed.
    #[error("Git backend I/O failed: {0}")]
    Io(#[source] io::Error),
    /// Backend emitted an invalid CGI response.
    #[error("invalid git-http-backend response: {0}")]
    InvalidBackendResponse(&'static str),
    /// Git metadata command failed.
    #[error("Git command failed: {0}")]
    Git(String),
}

#[cfg(test)]
mod tests {
    use super::{
        AuthorizationRequest, BackendEnvironment, GitAuthorizer, GitOperation,
        PostgresGitAuthorizer, Principal, authentication_error_response, backend_command,
        diff_refs, parse_basic_pat, parse_cgi_headers, pat_operation, validate_backend_path,
    };
    use async_trait::async_trait;
    use authz_domain::{
        AuthorizationDecision, AuthzError, GitRepositoryAuthorizer, GitRepositoryOperation,
    };
    use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
    use forge_domain::{CommitSha, GitRef, RepositoryId};
    use identity_domain::AuthenticatedIdentity;
    use pat_domain::{PersonalAccessToken, PersonalAccessTokenId};
    use std::{
        collections::{BTreeMap, BTreeSet},
        path::Path,
        sync::Arc,
    };
    use uuid::Uuid;

    struct UnexpectedRuntimeDelegate;

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
            parse_cgi_headers(b"Status: 403 Forbidden\r\nContent-Type: text/plain")
                .expect("headers");
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
}
