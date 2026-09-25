//! Owner-only Unix transport for runtime Git HTTP.

use axum::{
    Router,
    body::Body,
    extract::Request,
    http::{HeaderValue, StatusCode, header::AUTHORIZATION},
    middleware::Next,
    response::Response,
};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use git_http::GitHttpService;
use std::{
    io,
    os::unix::{
        fs::{FileTypeExt, MetadataExt, PermissionsExt},
        net::UnixListener as StdUnixListener,
    },
    path::{Path, PathBuf},
};
use tokio::net::UnixListener;

const SOCKET_MODE: u32 = 0o600;
const RUNTIME_USERNAME: &str = "heph-runtime";
const RUNTIME_TOKEN_PREFIX: &str = "heph_git_v1_";

/// A bound owner-only runtime Git listener.
pub struct RuntimeGitListener {
    listener: Option<UnixListener>,
    path: PathBuf,
    device: u64,
    inode: u64,
}

impl RuntimeGitListener {
    /// Binds a safe owner-only Unix socket, removing only an owned stale socket.
    pub fn bind(path: impl Into<PathBuf>) -> io::Result<Self> {
        let path = path.into();
        validate_socket_path(&path)?;
        remove_owned_stale_socket(&path)?;
        let listener = StdUnixListener::bind(&path)?;
        listener.set_nonblocking(true)?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(SOCKET_MODE))?;
        let metadata = std::fs::symlink_metadata(&path)?;
        let listener = UnixListener::from_std(listener)?;
        Ok(Self {
            listener: Some(listener),
            path,
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    }

    /// Serves the shared Git router until cancellation, then removes the
    /// socket only if its original inode is still present.
    pub async fn serve(
        mut self,
        router: Router,
        cancellation: tokio_util::sync::CancellationToken,
    ) -> io::Result<()> {
        axum::serve(
            self.listener
                .take()
                .expect("runtime Git listener is served only once"),
            router,
        )
        .with_graceful_shutdown(cancellation.cancelled_owned())
        .await
        .map_err(io::Error::other)
    }
}

impl Drop for RuntimeGitListener {
    fn drop(&mut self) {
        let Ok(metadata) = std::fs::symlink_metadata(&self.path) else {
            return;
        };
        if metadata.file_type().is_socket()
            && metadata.uid() == rustix::process::geteuid().as_raw()
            && metadata.dev() == self.device
            && metadata.ino() == self.inode
        {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// Builds the internal router over the shared Git service.
pub fn router(service: &GitHttpService) -> Router {
    service
        .clone()
        .router()
        .layer(axum::middleware::from_fn(admit_runtime_credential))
}

async fn admit_runtime_credential(request: Request<Body>, next: Next) -> Response {
    match request.headers().get(AUTHORIZATION) {
        Some(value) if is_runtime_credential(value) => next.run(request).await,
        Some(_) | None => runtime_admission_denied_response(),
    }
}

fn runtime_admission_denied_response() -> Response<Body> {
    Response::builder()
        .status(StatusCode::UNAUTHORIZED)
        .header("www-authenticate", r#"Basic realm="hephaestus-git""#)
        .body(Body::empty())
        .expect("static response is valid")
}

fn is_runtime_credential(value: &HeaderValue) -> bool {
    let Ok(value) = value.to_str() else {
        return false;
    };
    let Some(encoded) = value.strip_prefix("Basic ") else {
        return false;
    };
    let Ok(decoded) = BASE64_STANDARD.decode(encoded) else {
        return false;
    };
    let Ok(decoded) = std::str::from_utf8(&decoded) else {
        return false;
    };
    let Some((username, password)) = decoded.split_once(':') else {
        return false;
    };
    username == RUNTIME_USERNAME && password.starts_with(RUNTIME_TOKEN_PREFIX)
}

fn validate_socket_path(path: &Path) -> io::Result<()> {
    if !path.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "runtime Git socket path must be absolute",
        ));
    }
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "runtime Git socket has no parent",
        )
    })?;
    let metadata = std::fs::symlink_metadata(parent)?;
    if metadata.file_type().is_symlink()
        || !metadata.is_dir()
        || metadata.uid() != rustix::process::geteuid().as_raw()
        || metadata.permissions().mode() & 0o022 != 0
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "runtime Git socket parent is not a private owner directory",
        ));
    }
    Ok(())
}

fn remove_owned_stale_socket(path: &Path) -> io::Result<()> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if metadata.file_type().is_symlink() || !metadata.file_type().is_socket() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "runtime Git socket path is occupied by a non-socket",
        ));
    }
    if metadata.uid() != rustix::process::geteuid().as_raw() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "runtime Git socket is owned by another user",
        ));
    }
    match std::os::unix::net::UnixStream::connect(path) {
        Ok(stream) => {
            drop(stream);
            Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "runtime Git socket is live",
            ))
        }
        Err(error) if matches!(error.kind(), io::ErrorKind::ConnectionRefused) => {
            std::fs::remove_file(path)
        }
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::{RuntimeGitListener, admit_runtime_credential, is_runtime_credential};
    use axum::{
        Router,
        body::Body,
        http::{HeaderValue, Request, StatusCode},
        middleware,
        routing::{any, get},
    };
    use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
    use std::{
        fs,
        os::unix::fs::PermissionsExt,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
    };
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::{TcpListener, UnixStream},
        process::Command,
    };
    use tokio_util::sync::CancellationToken;
    use tower::ServiceExt;

    #[test]
    fn runtime_admission_rejects_missing_and_human_credentials() {
        assert!(!is_runtime_credential(&HeaderValue::from_static(
            "Basic abc"
        )));
        let human = BASE64_STANDARD.encode("human:pat");
        assert!(!is_runtime_credential(
            &HeaderValue::from_str(&format!("Basic {human}")).expect("header")
        ));
    }

    #[test]
    fn runtime_admission_accepts_only_the_runtime_prefix() {
        let credential = BASE64_STANDARD.encode("heph-runtime:heph_git_v1_token");
        assert!(is_runtime_credential(
            &HeaderValue::from_str(&format!("Basic {credential}")).expect("header")
        ));
    }

    #[tokio::test]
    async fn runtime_http_admission_denies_human_and_accepts_runtime_credentials() {
        let router = Router::new()
            .route("/", get(|| async { "ok" }))
            .layer(middleware::from_fn(admit_runtime_credential));
        let human = BASE64_STANDARD.encode("human:pat");
        let denied = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/")
                    .header("authorization", format!("Basic {human}"))
                    .body(Body::empty())
                    .expect("human request"),
            )
            .await
            .expect("human response");
        assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            denied.headers()["www-authenticate"],
            r#"Basic realm="hephaestus-git""#
        );

        let runtime = BASE64_STANDARD.encode("heph-runtime:heph_git_v1_test");
        let accepted = router
            .oneshot(
                Request::builder()
                    .uri("/")
                    .header("authorization", format!("Basic {runtime}"))
                    .body(Body::empty())
                    .expect("runtime request"),
            )
            .await
            .expect("runtime response");
        assert_eq!(accepted.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn native_git_retries_with_helper_after_runtime_basic_challenge() {
        let directory = tempfile::tempdir().expect("credential helper directory");
        let helper = directory.path().join("credential-helper");
        fs::write(
            &helper,
            b"#!/bin/sh\nprintf '%s\\n' 'username=heph-runtime' 'password=heph_git_v1_test'\n",
        )
        .expect("write credential helper");
        fs::set_permissions(&helper, fs::Permissions::from_mode(0o700))
            .expect("protect credential helper");

        let saw_authorization = Arc::new(AtomicBool::new(false));
        let saw_authorization_handler = Arc::clone(&saw_authorization);
        let router = Router::new()
            .fallback(any(move |request: Request<Body>| {
                let saw_authorization = Arc::clone(&saw_authorization_handler);
                async move {
                    if request.headers().contains_key("authorization") {
                        saw_authorization.store(true, Ordering::Release);
                    }
                    (
                        StatusCode::OK,
                        Body::from("001e# service=git-upload-pack\n0000"),
                    )
                }
            }))
            .layer(middleware::from_fn(admit_runtime_credential));
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind HTTP listener");
        let address = listener.local_addr().expect("listener address");
        let cancellation = CancellationToken::new();
        let server_cancellation = cancellation.clone();
        let server = tokio::spawn(async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(server_cancellation.cancelled_owned())
                .await
                .expect("serve HTTP listener");
        });

        let output = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            Command::new("/usr/bin/git")
                .args([
                    "-c",
                    "credential.helper=",
                    "-c",
                    &format!("credential.helper=!{}", helper.display()),
                    "-c",
                    "credential.useHttpPath=true",
                    "ls-remote",
                    &format!("http://{address}/repository"),
                ])
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env_remove("GIT_CONFIG_COUNT")
                .env_remove("GIT_CONFIG_KEY_0")
                .env_remove("GIT_CONFIG_VALUE_0")
                .env("GIT_TERMINAL_PROMPT", "0")
                .kill_on_drop(true)
                .output(),
        )
        .await
        .expect("native Git handshake timeout")
        .expect("run native Git handshake");
        cancellation.cancel();
        server.await.expect("server task");

        assert!(
            saw_authorization.load(Ordering::Acquire),
            "Git helper was not retried after challenge"
        );
        assert!(
            !output
                .stderr
                .windows(b"heph_git_v1_test".len())
                .any(|window| { window == b"heph_git_v1_test" })
        );
    }

    #[tokio::test]
    async fn unix_listener_serves_admitted_http_request() {
        let directory = tempfile::tempdir().expect("socket directory");
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700))
            .expect("private directory");
        let path = directory.path().join("runtime-git.sock");
        let listener = RuntimeGitListener::bind(&path).expect("bind listener");
        let router = Router::new()
            .route("/", get(|| async { "ok" }))
            .layer(middleware::from_fn(admit_runtime_credential));
        let cancellation = CancellationToken::new();
        let server_cancellation = cancellation.clone();
        let server = tokio::spawn(async move { listener.serve(router, server_cancellation).await });

        let credential = BASE64_STANDARD.encode("heph-runtime:heph_git_v1_test");
        let mut stream = UnixStream::connect(&path).await.expect("connect listener");
        stream
            .write_all(
                format!(
                    "GET / HTTP/1.1\r\nHost: runtime-git\r\nAuthorization: Basic {credential}\r\nConnection: close\r\n\r\n"
                )
                .as_bytes(),
            )
            .await
            .expect("write request");
        let mut response = Vec::new();
        stream
            .read_to_end(&mut response)
            .await
            .expect("read response");
        let response = String::from_utf8(response).expect("HTTP response");
        assert!(response.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(response.ends_with("\r\n\r\nok"));

        drop(stream);
        cancellation.cancel();
        server.await.expect("server task").expect("server shutdown");
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn socket_is_owner_only_and_stale_socket_is_replaced() {
        let directory = tempfile::tempdir().expect("socket directory");
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700))
            .expect("private directory");
        let path = directory.path().join("runtime-git.sock");
        let listener = RuntimeGitListener::bind(&path).expect("first bind");
        let mode = fs::symlink_metadata(&path)
            .expect("socket metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
        drop(listener);
        assert!(!path.exists());

        let stale = std::os::unix::net::UnixListener::bind(&path).expect("stale socket");
        drop(stale);
        let replacement = RuntimeGitListener::bind(&path).expect("replace stale socket");
        drop(replacement);
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn live_socket_is_not_replaced() {
        let directory = tempfile::tempdir().expect("socket directory");
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700))
            .expect("private directory");
        let path = directory.path().join("runtime-git.sock");
        let listener = RuntimeGitListener::bind(&path).expect("first bind");
        assert!(matches!(
            RuntimeGitListener::bind(&path),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists
        ));
        drop(listener);
    }
}
