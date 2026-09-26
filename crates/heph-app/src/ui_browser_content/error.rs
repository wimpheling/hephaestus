use axum::{
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::Serialize;
use thiserror::Error;

/// Redacted content errors. Every variant maps to generic no-store HTTP.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum UiContentError {
    /// Request syntax, path, host, cookie, or range was invalid.
    #[error("UI request is invalid")]
    InvalidRequest,
    /// No active generation or declaration matched.
    #[error("UI resource was not found")]
    NotFound,
    /// Child authentication or current authorization failed.
    #[error("UI request is unauthenticated")]
    Unauthenticated,
    /// A bounded body/range limit was exceeded.
    #[error("UI request is too large")]
    BodyTooLarge,
    /// A single range could not be satisfied.
    #[error("UI byte range is not satisfiable")]
    RangeNotSatisfiable,
    /// The guest attempted to set a browser cookie.
    #[error("UI guest response contained a cookie")]
    GuestSetCookie,
    /// The guest attempted to redirect the browser outside the declared UI path.
    #[error("UI guest response contained a redirect")]
    GuestRedirect,
    /// Adapter, artifact store, or gateway was unavailable.
    #[error("UI serving is unavailable")]
    Unavailable,
}

impl IntoResponse for UiContentError {
    fn into_response(self) -> Response {
        let status = match self {
            Self::InvalidRequest => StatusCode::BAD_REQUEST,
            Self::RangeNotSatisfiable => StatusCode::RANGE_NOT_SATISFIABLE,
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::Unauthenticated => StatusCode::UNAUTHORIZED,
            Self::BodyTooLarge => StatusCode::PAYLOAD_TOO_LARGE,
            Self::GuestSetCookie | Self::GuestRedirect => StatusCode::BAD_GATEWAY,
            Self::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
        };
        error_response(status, "ui_unavailable")
    }
}

#[derive(Debug, Serialize)]
struct ErrorBody {
    error: &'static str,
}

pub(super) fn error_response(status: StatusCode, code: &'static str) -> Response {
    let mut response = (status, axum::Json(ErrorBody { error: code })).into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response
}
