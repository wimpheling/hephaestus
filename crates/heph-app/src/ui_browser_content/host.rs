use super::{UiContentError, UiContentState};
use axum::{
    body::Body,
    http::{HeaderMap, Request, header},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use gateway_domain::HttpMethod;
use release_domain::ui_browser::UiBrowserSessionSecret;
use release_service::ui_browser_host::{
    UI_CHILD_COOKIE, UiGenerationHost, UiNamespace, UiPublicPort,
};
use release_service::ui_browser_serving::{
    ActiveUiGenerationHost, UiHostLookupError, UiServingError,
};
pub(super) const fn map_serving_error(error: UiServingError) -> UiContentError {
    match error {
        UiServingError::Unauthenticated => UiContentError::Unauthenticated,
        UiServingError::Unavailable => UiContentError::Unavailable,
    }
}

pub(super) async fn parse_and_resolve_host(
    state: &UiContentState,
    authority: String,
) -> Result<UiGenerationHost, UiContentError> {
    let host = UiGenerationHost::parse(&authority, &state.namespace, state.public_port)
        .map_err(|_| UiContentError::NotFound)?;
    match state
        .host_resolver
        .resolve_active_generation_host(host)
        .await
        .map_err(|error| match error {
            UiHostLookupError::Unavailable => UiContentError::Unavailable,
        })? {
        Some(ActiveUiGenerationHost { generation_id }) if generation_id == host.generation_id() => {
            Ok(host)
        }
        Some(_) | None => Err(UiContentError::NotFound),
    }
}

pub(super) fn request_authority(request: &Request<Body>) -> Result<String, UiContentError> {
    if request.uri().scheme().is_some()
        || request.uri().authority().is_some()
        || request.headers().get_all(header::HOST).iter().count() != 1
    {
        return Err(UiContentError::InvalidRequest);
    }
    request
        .headers()
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
        .ok_or(UiContentError::InvalidRequest)
}

pub(super) fn require_unsafe_origin(
    request: &Request<Body>,
    host: &UiGenerationHost,
    method: HttpMethod,
    namespace: &UiNamespace,
    public_port: UiPublicPort,
) -> Result<(), UiContentError> {
    let origin_count = request.headers().get_all(header::ORIGIN).iter().count();
    if origin_count == 0 && matches!(method, HttpMethod::Get | HttpMethod::Head) {
        return Ok(());
    }
    if origin_count != 1 {
        return Err(UiContentError::InvalidRequest);
    }
    let expected = format!("https://{}", host.authority(namespace, public_port));
    let origin = request
        .headers()
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        .ok_or(UiContentError::InvalidRequest)?;
    if origin != expected {
        return Err(UiContentError::InvalidRequest);
    }
    if !matches!(method, HttpMethod::Get | HttpMethod::Head)
        && request
            .headers()
            .get("sec-fetch-site")
            .is_some_and(|value| value.as_bytes() != b"same-origin")
    {
        return Err(UiContentError::InvalidRequest);
    }
    Ok(())
}

pub(super) fn parse_child_cookie(
    headers: &HeaderMap,
) -> Result<UiBrowserSessionSecret, UiContentError> {
    let mut found = None;
    for value in &headers.get_all(header::COOKIE) {
        let value = value
            .to_str()
            .map_err(|_| UiContentError::Unauthenticated)?;
        for pair in value.split(';') {
            let pair = pair.trim();
            let Some((name, encoded)) = pair.split_once('=') else {
                return Err(UiContentError::Unauthenticated);
            };
            if name != UI_CHILD_COOKIE {
                continue;
            }
            if found.is_some() || !is_secret_text(encoded) {
                return Err(UiContentError::Unauthenticated);
            }
            let bytes = URL_SAFE_NO_PAD
                .decode(encoded)
                .map_err(|_| UiContentError::Unauthenticated)?;
            let bytes: [u8; 32] = bytes
                .try_into()
                .map_err(|_| UiContentError::Unauthenticated)?;
            found = Some(UiBrowserSessionSecret::from_bytes(bytes));
        }
    }
    found.ok_or(UiContentError::Unauthenticated)
}

pub(super) fn is_secret_text(value: &str) -> bool {
    value.len() == 43
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}
