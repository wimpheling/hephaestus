use axum::{
    body::Body,
    http::{HeaderValue, Response, StatusCode},
    response::IntoResponse,
};
use std::io;
use std::path::PathBuf;

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
pub fn domain(error: impl std::fmt::Display) -> GitHttpError {
    GitHttpError::Git(error.to_string())
}

pub fn error_response(status: StatusCode, message: &str) -> Response<Body> {
    (
        status,
        [(http::header::CONTENT_TYPE, "application/json")],
        serde_json::to_string(&serde_json::json!({"error": message}))
            .unwrap_or_else(|_| String::from(r#"{"error":"internal error"}"#)),
    )
        .into_response()
}

pub fn authentication_error_response(message: &str) -> Response<Body> {
    let mut response = error_response(StatusCode::UNAUTHORIZED, message);
    response.headers_mut().insert(
        http::header::WWW_AUTHENTICATE,
        HeaderValue::from_static(r#"Basic realm="hephaestus-git""#),
    );
    response
}

pub fn service_error(error: impl std::fmt::Display) -> Response<Body> {
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
