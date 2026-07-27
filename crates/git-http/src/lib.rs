//! Authorized, bounded, streaming Git smart-HTTP transport.

use async_trait::async_trait;
use axum::{
    Router,
    body::Body,
    extract::{Path as AxumPath, Query, State},
    http::{HeaderMap, HeaderName, HeaderValue, Request, Response, StatusCode},
    response::IntoResponse,
    routing::{get, post},
};
use bytes::{Bytes, BytesMut};
use forge_domain::{CommitSha, GitRef, ReceiveId, RefUpdate, RepositoryId};
use forge_service::{ForgeRepositoryError, GitStorage, PgForgeRepository};
use futures_util::StreamExt;
use std::{
    collections::{BTreeMap, HashMap},
    io,
    path::PathBuf,
    process::Stdio,
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::{ChildStdout, Command},
    sync::{Mutex, OwnedMutexGuard, mpsc},
};
use tokio_stream::wrappers::ReceiverStream;

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

/// Authenticated principal returned by an authorizer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Principal {
    /// Stable provider-neutral principal name.
    pub name: String,
}

/// Owned authorization input.
#[derive(Debug, Clone)]
pub struct AuthorizationRequest {
    /// Repository being accessed.
    pub repository_id: RepositoryId,
    /// Requested operation.
    pub operation: GitOperation,
    /// Opaque HTTP authorization value, when supplied.
    pub credential: Option<String>,
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
    async fn authorize(
        &self,
        request: &AuthorizationRequest,
    ) -> Result<Principal, AuthorizationError>;
}

/// Development authorizer that always returns one configured principal.
#[derive(Debug, Clone)]
pub struct FixedPrincipalAuthorizer {
    principal: Principal,
}

impl FixedPrincipalAuthorizer {
    /// Creates the development authorizer.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            principal: Principal { name: name.into() },
        }
    }
}

#[async_trait]
impl GitAuthorizer for FixedPrincipalAuthorizer {
    async fn authorize(
        &self,
        _request: &AuthorizationRequest,
    ) -> Result<Principal, AuthorizationError> {
        Ok(self.principal.clone())
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
    authorizer: Arc<dyn GitAuthorizer>,
    backend: PathBuf,
    limits: GitHttpLimits,
    receive_locks: Arc<Mutex<HashMap<RepositoryId, Arc<Mutex<()>>>>>,
}

impl GitHttpService {
    /// Creates a service around a resolved `git-http-backend` executable.
    #[must_use]
    pub fn new(
        repository: Arc<PgForgeRepository>,
        storage: Arc<GitStorage>,
        authorizer: Arc<dyn GitAuthorizer>,
        backend: PathBuf,
        limits: GitHttpLimits,
    ) -> Self {
        Self {
            repository,
            storage,
            authorizer,
            backend,
            limits,
            receive_locks: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Builds Axum routes rooted at `/{repository_id}`.
    pub fn router(self) -> Router {
        Router::new()
            .route("/{repository}/info/refs", get(info_refs))
            .route("/{repository}/git-upload-pack", post(upload_pack))
            .route("/{repository}/git-receive-pack", post(receive_pack))
            .with_state(Arc::new(self))
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
    request: Request<Body>,
    receive: bool,
) -> Response<Body> {
    let repository_id = match GitStorage::parse_route(&route) {
        Ok(id) => id,
        Err(error) => return error_response(StatusCode::BAD_REQUEST, &error.to_string()),
    };
    let repository = match service.repository.get_repository(repository_id).await {
        Ok(repository) => repository,
        Err(ForgeRepositoryError::RepositoryNotFound(_)) => {
            return error_response(StatusCode::NOT_FOUND, "repository was not found");
        }
        Err(error) => return service_error(error),
    };
    let credential = request
        .headers()
        .get(http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let principal = match service
        .authorizer
        .authorize(&AuthorizationRequest {
            repository_id,
            operation,
            credential,
        })
        .await
    {
        Ok(principal) => principal,
        Err(error) => return error_response(StatusCode::FORBIDDEN, &error.to_string()),
    };
    if let Some(content_length) = request
        .headers()
        .get(http::header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        && content_length > service.limits.max_request_bytes
    {
        return error_response(StatusCode::PAYLOAD_TOO_LARGE, "Git request exceeds limit");
    }

    let receive_id = receive.then(ReceiveId::new);
    let receive_guard = if receive {
        Some(service.lock_receive(repository_id).await)
    } else {
        None
    };
    let before = if receive {
        match snapshot_refs(service.storage.repository_path(repository_id)).await {
            Ok(refs) => Some(refs),
            Err(error) => return service_error(error),
        }
    } else {
        None
    };
    let mut command = Command::new(&service.backend);
    command
        .env_clear()
        .env("GIT_PROJECT_ROOT", service.storage.root())
        .env("GIT_HTTP_EXPORT_ALL", "1")
        .env("PATH_INFO", format!("/{repository_id}.git/{endpoint}"))
        .env("REQUEST_METHOD", request.method().as_str())
        .env("QUERY_STRING", query.as_deref().unwrap_or(""))
        .env("GATEWAY_INTERFACE", "CGI/1.1")
        .env("SERVER_PROTOCOL", "HTTP/1.1")
        .env("REMOTE_USER", &principal.name)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    copy_request_header(
        &request,
        &mut command,
        http::header::CONTENT_TYPE,
        "CONTENT_TYPE",
    );
    copy_request_header(
        &request,
        &mut command,
        http::header::CONTENT_LENGTH,
        "CONTENT_LENGTH",
    );
    copy_request_header_name(&request, &mut command, "git-protocol", "GIT_PROTOCOL");

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

    let max_request_bytes = service.limits.max_request_bytes;
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
    let principal_name = principal.name.clone();
    let timeout = service.limits.transaction_timeout;
    tokio::spawn(async move {
        let _receive_guard = receive_guard;
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
                        .accept_receive(
                            &repository_for_receive,
                            receive_id,
                            &principal_name,
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

fn copy_request_header(
    request: &Request<Body>,
    command: &mut Command,
    header: HeaderName,
    environment: &str,
) {
    if let Some(value) = request
        .headers()
        .get(header)
        .and_then(|value| value.to_str().ok())
    {
        command.env(environment, value);
    }
}

fn copy_request_header_name(
    request: &Request<Body>,
    command: &mut Command,
    header: &str,
    environment: &str,
) {
    if let Some(value) = request
        .headers()
        .get(header)
        .and_then(|value| value.to_str().ok())
    {
        command.env(environment, value);
    }
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

fn service_error(error: impl std::fmt::Display) -> Response<Body> {
    tracing::warn!(%error, "Git HTTP request failed");
    error_response(StatusCode::INTERNAL_SERVER_ERROR, &error.to_string())
}

/// Native Git HTTP adapter failure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum GitHttpError {
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
    use super::{diff_refs, parse_cgi_headers};
    use forge_domain::{CommitSha, GitRef};
    use std::collections::BTreeMap;

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
}
