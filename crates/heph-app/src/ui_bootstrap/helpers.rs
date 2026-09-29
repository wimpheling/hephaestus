use axum::{
    body::{Body, to_bytes},
    http::{HeaderMap, HeaderValue, Request, StatusCode, header},
    response::{IntoResponse, Response},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use identity_domain::RequestId;
use release_domain::ui_browser::{UiBrowserHandoffSecret, UiBrowserSessionSecret};
use release_service::ui_browser_host::{
    UI_HANDOFF_FRAGMENT_LENGTH, UiGenerationHost, is_handoff_fragment,
};
use release_service::ui_browser_serving::{ActiveUiGenerationHost, UiHostLookupError};
use release_service::{ExchangeUiBrowserHandoff, UiBrowserHandoffError};
use serde::Serialize;
use std::str;
use time::OffsetDateTime;

use super::config::{MAX_HANDOFF_BODY_BYTES, UiBootstrapConfig, UiBootstrapState};

pub(super) async fn resolve_host(
    state: &UiBootstrapState,
    authority: String,
) -> Result<Option<UiGenerationHost>, StatusCode> {
    let host = UiGenerationHost::parse(
        &authority,
        state.config.namespace(),
        state.config.public_port(),
    )
    .map_err(|_| StatusCode::NOT_FOUND)?;
    match state
        .host_resolver
        .resolve_active_generation_host(host)
        .await
    {
        Ok(Some(ActiveUiGenerationHost { generation_id }))
            if generation_id == host.generation_id() =>
        {
            Ok(Some(host))
        }
        Ok(Some(_) | None) => Ok(None),
        Err(UiHostLookupError::Unavailable) => Err(StatusCode::SERVICE_UNAVAILABLE),
    }
}

pub(super) fn request_authority(request: &Request<Body>) -> Result<String, StatusCode> {
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

pub(super) async fn exchange(
    state: &UiBootstrapState,
    request_id: RequestId,
    request: Request<Body>,
    host: UiGenerationHost,
) -> Result<ExchangeCreated, UiBrowserHandoffError> {
    let body = to_bytes(request.into_body(), MAX_HANDOFF_BODY_BYTES)
        .await
        .map_err(|_| UiBrowserHandoffError::InvalidOrExpired)?;
    let text = str::from_utf8(&body).map_err(|_| UiBrowserHandoffError::InvalidOrExpired)?;
    if !is_handoff_fragment(text) || text.len() != UI_HANDOFF_FRAGMENT_LENGTH {
        return Err(UiBrowserHandoffError::InvalidOrExpired);
    }
    let bytes = URL_SAFE_NO_PAD
        .decode(text)
        .map_err(|_| UiBrowserHandoffError::InvalidOrExpired)?;
    let bytes: [u8; 32] = bytes
        .try_into()
        .map_err(|_| UiBrowserHandoffError::InvalidOrExpired)?;
    let handoff_secret = UiBrowserHandoffSecret::from_bytes(bytes);
    let child_secret = UiBrowserSessionSecret::random();
    let child_cookie_value = URL_SAFE_NO_PAD.encode(child_secret.as_bytes());
    let created = state
        .sessions
        .exchange_ui_browser_handoff(ExchangeUiBrowserHandoff {
            request_id,
            handoff_secret,
            expected_generation_id: host.generation_id(),
            child_secret,
        })
        .await?;
    let expires_at = created.context.expires_at;
    Ok(ExchangeCreated {
        context: created.context,
        expires_at,
        child_cookie_value,
    })
}

/// A private transport-only value. The raw child secret is consumed once to
/// build `Set-Cookie` and never crosses the JSON response boundary.
pub(super) struct ExchangeCreated {
    pub(super) context: release_service::UiBrowserSessionContext,
    pub(super) expires_at: OffsetDateTime,
    pub(super) child_cookie_value: String,
}

pub(super) fn exchange_error_response(error: UiBrowserHandoffError) -> Response {
    match error {
        UiBrowserHandoffError::PermissionDenied
        | UiBrowserHandoffError::InvalidRoute
        | UiBrowserHandoffError::InvalidOrExpired => {
            error_response(StatusCode::UNAUTHORIZED, "bootstrap_unauthorized")
        }
        UiBrowserHandoffError::Unavailable => {
            error_response(StatusCode::SERVICE_UNAVAILABLE, "bootstrap_unavailable")
        }
    }
}

pub(super) fn error_response(status: StatusCode, code: &'static str) -> Response {
    let mut response = (status, axum::Json(ErrorResponse { error: code })).into_response();
    add_no_store(response.headers_mut());
    insert_header(
        response.headers_mut(),
        header::X_CONTENT_TYPE_OPTIONS,
        "nosniff",
    );
    response
}

#[derive(Debug, Serialize)]
struct ErrorResponse {
    error: &'static str,
}

pub(super) fn exact_origin_matches(
    request: &Request<Body>,
    host: &UiGenerationHost,
    config: &UiBootstrapConfig,
) -> bool {
    if request.headers().get_all(header::ORIGIN).iter().count() != 1 {
        return false;
    }
    let Some(origin) = request.headers().get(header::ORIGIN) else {
        return false;
    };
    let Ok(origin) = origin.to_str() else {
        return false;
    };
    origin
        == format!(
            "https://{}",
            host.authority(config.namespace(), config.public_port())
        )
}

pub(super) fn child_max_age(expires_at: OffsetDateTime) -> Option<i64> {
    let seconds = (expires_at - OffsetDateTime::now_utc()).whole_seconds();
    (1..=12 * 60 * 60).contains(&seconds).then_some(seconds)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Theme {
    Light,
    Dark,
}

impl Theme {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    const fn query_suffix(self) -> &'static str {
        match self {
            Self::Light => "?heph_theme=light",
            Self::Dark => "?heph_theme=dark",
        }
    }
}

pub(super) fn parse_theme(query: Option<&str>) -> Result<ThemeOption, ()> {
    let mut theme = None;
    for pair in query
        .unwrap_or_default()
        .split('&')
        .filter(|pair| !pair.is_empty())
    {
        let Some((key, value)) = pair.split_once('=') else {
            if pair == "heph_theme" {
                return Err(());
            }
            continue;
        };
        if key != "heph_theme" {
            continue;
        }
        if theme.is_some() {
            return Err(());
        }
        theme = Some(match value {
            "light" => Theme::Light,
            "dark" => Theme::Dark,
            _ => return Err(()),
        });
    }
    Ok(ThemeOption(theme))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ThemeOption(pub(super) Option<Theme>);

impl ThemeOption {
    pub(super) fn as_str(self) -> Option<&'static str> {
        self.0.map(Theme::as_str)
    }

    pub(super) fn query_suffix(self) -> &'static str {
        self.0.map_or("", Theme::query_suffix)
    }
}

pub(super) fn add_no_store(headers: &mut HeaderMap) {
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
}

pub(super) fn insert_header(headers: &mut HeaderMap, name: header::HeaderName, value: &str) {
    if let Ok(value) = HeaderValue::from_str(value) {
        headers.insert(name, value);
    }
}

pub(super) fn bootstrap_html(nonce: &str, platform_origin: &str) -> String {
    let platform_meta = format!(
        "<meta name=\"heph-platform-origin\" content=\"{}\">",
        escape_attribute(platform_origin)
    );
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><meta name=\"referrer\" content=\"no-referrer\">{platform_meta}</head><body><p>Opening Heph UI…</p><script nonce=\"{nonce}\">{BOOTSTRAP_SCRIPT}</script></body></html>"
    )
}

const BOOTSTRAP_SCRIPT: &str = include_str!("../ui_bootstrap.js");

fn escape_attribute(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
