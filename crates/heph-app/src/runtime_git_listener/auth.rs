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

const RUNTIME_USERNAME: &str = "heph-runtime";
const RUNTIME_TOKEN_PREFIX: &str = "heph_git_v1_";

/// Builds the internal router over the shared Git service.
pub fn router(service: &GitHttpService) -> Router {
    service
        .clone()
        .router()
        .layer(axum::middleware::from_fn(admit_runtime_credential))
}

pub async fn admit_runtime_credential(request: Request<Body>, next: Next) -> Response {
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

pub fn is_runtime_credential(value: &HeaderValue) -> bool {
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
