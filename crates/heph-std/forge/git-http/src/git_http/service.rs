use crate::{
    GitAuthenticator, GitAuthorizer, GitHttpError, GitOperation, errors::error_response,
    execution::execute,
};
use axum::{
    Router,
    body::Body,
    extract::{Path as AxumPath, Query, State},
    http::{Request, Response, StatusCode},
    routing::{get, post},
};
use forge_domain::RepositoryId;
use forge_postgres::PgForgeRepository;
use forge_service::GitStorage;
use identity_domain::AuthenticatedIdentity;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::sync::{Mutex, OwnedMutexGuard};

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

/// One canonical Git endpoint selected by a trusted browser adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthenticatedHumanGitEndpoint {
    /// Clone advertisement.
    CloneInfoRefs,
    /// Push advertisement.
    PushInfoRefs,
    /// Fetch object transfer.
    UploadPack,
    /// Push object transfer and receive persistence.
    ReceivePack,
}

impl AuthenticatedHumanGitEndpoint {
    // This helper is shared only by the crate-root execution boundary.
    #[allow(clippy::redundant_pub_crate)]
    pub(super) fn parameters(self) -> (GitOperation, &'static str, Option<String>, bool) {
        match self {
            Self::CloneInfoRefs => (
                GitOperation::Clone,
                "info/refs",
                Some(String::from("service=git-upload-pack")),
                false,
            ),
            Self::PushInfoRefs => (
                GitOperation::Push,
                "info/refs",
                Some(String::from("service=git-receive-pack")),
                false,
            ),
            Self::UploadPack => (GitOperation::Fetch, "git-upload-pack", None, false),
            Self::ReceivePack => (GitOperation::Push, "git-receive-pack", None, true),
        }
    }

    /// Returns the native endpoint name used by `git-http-backend`.
    #[must_use]
    pub const fn backend_endpoint(self) -> &'static str {
        match self {
            Self::CloneInfoRefs | Self::PushInfoRefs => "info/refs",
            Self::UploadPack => "git-upload-pack",
            Self::ReceivePack => "git-receive-pack",
        }
    }
}

/// One browser-origin Git request whose human identity was verified by the
/// enclosing UI authority boundary.
pub struct AuthenticatedHumanGitRequest {
    /// Exact repository selected by the trusted adapter.
    pub repository_id: RepositoryId,
    /// Canonical endpoint and operation requested by the adapter.
    pub endpoint: AuthenticatedHumanGitEndpoint,
    /// Sanitized HTTP request body and transport headers.
    pub request: Request<Body>,
    /// Human identity selected by live UI authorization.
    pub identity: AuthenticatedIdentity,
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
    pub(super) repository: Arc<PgForgeRepository>,
    pub(super) storage: Arc<GitStorage>,
    pub(super) authenticator: Arc<dyn GitAuthenticator>,
    pub(super) authorizer: Arc<dyn GitAuthorizer>,
    pub(super) backend: PathBuf,
    pub(super) limits: GitHttpLimits,
    pub(super) receive_locks: Arc<Mutex<HashMap<RepositoryId, Arc<Mutex<()>>>>>,
    pub(super) runtime_receive_hook: Option<PathBuf>,
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

    /// Returns the configured request and response ceilings for trusted
    /// adapters that perform transport-level admission before execution.
    #[must_use]
    pub const fn limits(&self) -> &GitHttpLimits {
        &self.limits
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

    // This lock is shared only by the crate-root execution boundary.
    #[allow(clippy::redundant_pub_crate)]
    pub(super) async fn lock_receive(&self, repository_id: RepositoryId) -> OwnedMutexGuard<()> {
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

// This validation helper is shared with the crate-local test boundary.
#[allow(clippy::redundant_pub_crate)]
pub(super) fn validate_backend_path(backend: &Path) -> Result<(), GitHttpError> {
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
