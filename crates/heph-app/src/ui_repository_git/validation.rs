use axum::{
    body::Body,
    http::{Request, StatusCode, header},
    response::Response,
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use git_http::AuthenticatedHumanGitEndpoint;
use release_domain::ui_browser::UiBrowserSessionSecret;
use release_service::{
    UiNamespace, UiPublicPort, UiRepositoryGitOperation,
    ui_browser_host::{UI_CHILD_COOKIE, UiGenerationHost},
};

use super::common::GIT_ACTOR_HEADER;

pub fn request_authority(request: &Request<Body>) -> Result<String, StatusCode> {
    if request.uri().scheme().is_some()
        || request.uri().authority().is_some()
        || request.headers().get_all(header::HOST).iter().count() != 1
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    request
        .headers()
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
        .ok_or(StatusCode::BAD_REQUEST)
}

pub fn require_same_origin(
    request: &Request<Body>,
    host: UiGenerationHost,
    namespace: &UiNamespace,
    public_port: UiPublicPort,
    unsafe_request: bool,
) -> Result<(), StatusCode> {
    let expected = format!("https://{}", host.authority(namespace, public_port));
    let origins = request.headers().get_all(header::ORIGIN);
    if origins.iter().count() == 0 && !unsafe_request {
        return Ok(());
    }
    if origins.iter().count() != 1
        || origins.iter().next().and_then(|value| value.to_str().ok()) != Some(expected.as_str())
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    if unsafe_request
        && request
            .headers()
            .get("sec-fetch-site")
            .is_some_and(|value| value.as_bytes() != b"same-origin")
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    Ok(())
}

pub fn parse_child_cookie(request: &Request<Body>) -> Result<UiBrowserSessionSecret, StatusCode> {
    let mut found = None;
    for value in &request.headers().get_all(header::COOKIE) {
        let value = value.to_str().map_err(|_| StatusCode::UNAUTHORIZED)?;
        for pair in value.split(';') {
            let pair = pair.trim();
            let Some((name, encoded)) = pair.split_once('=') else {
                return Err(StatusCode::UNAUTHORIZED);
            };
            if name != UI_CHILD_COOKIE {
                continue;
            }
            if found.is_some()
                || encoded.len() != 43
                || !encoded
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
            {
                return Err(StatusCode::UNAUTHORIZED);
            }
            let bytes = URL_SAFE_NO_PAD
                .decode(encoded)
                .map_err(|_| StatusCode::UNAUTHORIZED)?;
            let bytes: [u8; 32] = bytes.try_into().map_err(|_| StatusCode::UNAUTHORIZED)?;
            found = Some(UiBrowserSessionSecret::from_bytes(bytes));
        }
    }
    found.ok_or(StatusCode::UNAUTHORIZED)
}

pub fn error_response(status: StatusCode, message: &'static str) -> Response<Body> {
    let mut response = Response::new(Body::from(format!("{{\"error\":\"{message}\"}}")));
    *response.status_mut() = status;
    response
}

pub(super) const fn authority_operation_for(
    endpoint: AuthenticatedHumanGitEndpoint,
) -> UiRepositoryGitOperation {
    match endpoint {
        AuthenticatedHumanGitEndpoint::CloneInfoRefs
        | AuthenticatedHumanGitEndpoint::UploadPack => UiRepositoryGitOperation::Read,
        AuthenticatedHumanGitEndpoint::PushInfoRefs
        | AuthenticatedHumanGitEndpoint::ReceivePack => UiRepositoryGitOperation::Write,
    }
}

pub(super) fn actor_header(
    mut response: Response<Body>,
    actor_id: identity_domain::UserId,
    endpoint: AuthenticatedHumanGitEndpoint,
) -> Response<Body> {
    if endpoint.backend_endpoint() == "info/refs" {
        if let Ok(value) = axum::http::HeaderValue::from_str(&actor_id.to_string()) {
            response.headers_mut().insert(GIT_ACTOR_HEADER, value);
        }
    }
    response
}
