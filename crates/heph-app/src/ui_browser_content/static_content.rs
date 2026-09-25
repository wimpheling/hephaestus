use super::{UiContentError, UiContentState, UiHttpRequest, apply_platform_csp, hex_bytes};
use axum::{
    body::Body,
    http::{HeaderMap, HeaderValue, Request, StatusCode, header},
    response::Response,
};
use gateway_domain::HttpMethod;
use release_domain::{ContentHash, ui::UiCachePolicy};
use release_service::ui_browser_serving::UiStaticArtifactProjection;
use std::sync::Arc;
pub(super) async fn serve_static(
    state: &UiContentState,
    request_headers: &StaticRequestHeaders,
    http_request: &UiHttpRequest,
    artifact: UiStaticArtifactProjection,
) -> Result<Response, UiContentError> {
    if !matches!(http_request.method, HttpMethod::Get | HttpMethod::Head) {
        return Err(UiContentError::NotFound);
    }
    let cache_policy = artifact.cache_policy;
    if artifact.size_bytes > state.max_static_bytes {
        return Err(UiContentError::Unavailable);
    }
    let permit = state
        .static_read_permits
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| UiContentError::Unavailable)?;
    let store = Arc::clone(&state.artifacts);
    let read = tokio::time::timeout(
        state.deadline,
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            store.read_verified(
                artifact.storage_key,
                artifact.content_hash,
                artifact.size_bytes,
                artifact.size_bytes,
            )
        }),
    )
    .await
    .map_err(|_| UiContentError::Unavailable)?
    .map_err(|_| UiContentError::Unavailable)?
    .map_err(|_| UiContentError::Unavailable)?;
    static_response(
        request_headers,
        http_request,
        artifact.content_hash,
        artifact.media_type.as_str(),
        cache_policy,
        &state.platform_csp,
        read,
    )
}

#[derive(Debug, Default)]
pub(super) struct StaticRequestHeaders {
    if_none_match: Option<String>,
    range: Option<String>,
    if_range: Option<String>,
}

impl StaticRequestHeaders {
    pub(super) fn from_request(request: &Request<Body>) -> Self {
        let header_value = |name| {
            request
                .headers()
                .get(name)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned)
        };
        Self {
            if_none_match: header_value(header::IF_NONE_MATCH),
            range: header_value(header::RANGE),
            if_range: header_value(header::IF_RANGE),
        }
    }
}

pub(super) fn static_response(
    request_headers: &StaticRequestHeaders,
    http_request: &UiHttpRequest,
    hash: ContentHash,
    media_type: &str,
    cache_policy: UiCachePolicy,
    platform_csp: &HeaderValue,
    bytes: Vec<u8>,
) -> Result<Response, UiContentError> {
    let etag = format!("\"{}\"", hex_bytes(hash.as_bytes()));
    if request_headers
        .if_none_match
        .as_deref()
        .is_some_and(|value| value == etag)
    {
        let mut response = Response::new(Body::empty());
        *response.status_mut() = StatusCode::NOT_MODIFIED;
        common_static_headers(
            response.headers_mut(),
            media_type,
            &etag,
            cache_policy,
            platform_csp,
        );
        return Ok(response);
    }
    let range = match (&request_headers.range, &request_headers.if_range) {
        (Some(_), Some(if_range)) if if_range != &etag => None,
        (Some(value), _) => match parse_single_range(value, bytes.len() as u64) {
            Ok(range) => Some(range),
            Err(UiContentError::RangeNotSatisfiable) => {
                return Ok(range_not_satisfiable(
                    bytes.len() as u64,
                    media_type,
                    &etag,
                    cache_policy,
                    platform_csp,
                ));
            }
            Err(error) => return Err(error),
        },
        (None, _) => None,
    };
    let (status, body, content_range) = match range {
        Some((start, end)) => (
            StatusCode::PARTIAL_CONTENT,
            bytes[usize::try_from(start).map_err(|_| UiContentError::Unavailable)?
                ..=usize::try_from(end).map_err(|_| UiContentError::Unavailable)?]
                .to_vec(),
            Some(format!("bytes {start}-{end}/{}", bytes.len())),
        ),
        None => (StatusCode::OK, bytes, None),
    };
    let body_len = body.len();
    let is_head = http_request.method == HttpMethod::Head;
    let mut response = if is_head {
        Response::new(Body::empty())
    } else {
        Response::new(Body::from(body))
    };
    *response.status_mut() = status;
    common_static_headers(
        response.headers_mut(),
        media_type,
        &etag,
        cache_policy,
        platform_csp,
    );
    response.headers_mut().insert(
        header::CONTENT_LENGTH,
        HeaderValue::from_str(&body_len.to_string()).map_err(|_| UiContentError::Unavailable)?,
    );
    response
        .headers_mut()
        .insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    if let Some(content_range) = content_range {
        response.headers_mut().insert(
            header::CONTENT_RANGE,
            HeaderValue::from_str(&content_range).map_err(|_| UiContentError::Unavailable)?,
        );
    }
    Ok(response)
}

pub(super) fn range_not_satisfiable(
    length: u64,
    media_type: &str,
    etag: &str,
    cache_policy: UiCachePolicy,
    platform_csp: &HeaderValue,
) -> Response {
    let mut response = Response::new(Body::empty());
    *response.status_mut() = StatusCode::RANGE_NOT_SATISFIABLE;
    common_static_headers(
        response.headers_mut(),
        media_type,
        etag,
        cache_policy,
        platform_csp,
    );
    response
        .headers_mut()
        .insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    response.headers_mut().insert(
        header::CONTENT_RANGE,
        HeaderValue::from_str(&format!("bytes */{length}")).expect("bounded content range header"),
    );
    response
}

pub(super) fn common_static_headers(
    headers: &mut HeaderMap,
    media_type: &str,
    etag: &str,
    cache_policy: UiCachePolicy,
    platform_csp: &HeaderValue,
) {
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(media_type)
            .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream")),
    );
    headers.insert(header::ETAG, HeaderValue::from_str(etag).expect("hex ETag"));
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    if matches!(cache_policy, UiCachePolicy::NoStore) {
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        headers.insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    }
    apply_platform_csp(headers, platform_csp);
}

pub(super) fn parse_single_range(value: &str, length: u64) -> Result<(u64, u64), UiContentError> {
    if length == 0 || !value.starts_with("bytes=") {
        return Err(UiContentError::RangeNotSatisfiable);
    }
    let range = &value[6..];
    if range.is_empty() || range.contains(',') {
        return Err(UiContentError::RangeNotSatisfiable);
    }
    let (start, end) = range
        .split_once('-')
        .ok_or(UiContentError::RangeNotSatisfiable)?;
    if start.is_empty() {
        let suffix = end
            .parse::<u64>()
            .map_err(|_| UiContentError::RangeNotSatisfiable)?;
        if suffix == 0 {
            return Err(UiContentError::RangeNotSatisfiable);
        }
        let start = length.saturating_sub(suffix);
        return Ok((start, length - 1));
    }
    let start = start
        .parse::<u64>()
        .map_err(|_| UiContentError::RangeNotSatisfiable)?;
    if start >= length {
        return Err(UiContentError::RangeNotSatisfiable);
    }
    let end = if end.is_empty() {
        length - 1
    } else {
        end.parse::<u64>()
            .map_err(|_| UiContentError::RangeNotSatisfiable)?
            .min(length - 1)
    };
    if end < start {
        return Err(UiContentError::RangeNotSatisfiable);
    }
    Ok((start, end))
}
