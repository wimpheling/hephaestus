//! Owner-only Unix transport for runtime Git HTTP.

#[path = "runtime_git_listener/auth.rs"]
mod auth;
#[path = "runtime_git_listener/listener.rs"]
mod listener;

pub use auth::router;
#[cfg(test)]
pub use auth::{admit_runtime_credential, is_runtime_credential};
pub use listener::RuntimeGitListener;

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
