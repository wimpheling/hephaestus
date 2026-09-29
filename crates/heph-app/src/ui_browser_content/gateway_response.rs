use super::{MAX_UI_RESPONSE_BODY_BYTES, UiContentError, UiGatewayResponse};
use axum::{
    body::Body,
    http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header},
    response::Response,
};
use gateway_domain::HttpMethod;

pub(super) fn redirect_response(
    location: &release_service::UiBrowserHttpPath,
    query: Option<&str>,
    platform_csp: &HeaderValue,
) -> Result<Response, UiContentError> {
    let location = query.map_or_else(
        || location.as_str().to_owned(),
        |query| format!("{}?{query}", location.as_str()),
    );
    let mut response = Response::new(Body::empty());
    *response.status_mut() = StatusCode::TEMPORARY_REDIRECT;
    response.headers_mut().insert(
        header::LOCATION,
        HeaderValue::from_str(&location).map_err(|_| UiContentError::Unavailable)?,
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    apply_platform_csp(response.headers_mut(), platform_csp);
    Ok(response)
}

pub(super) fn gateway_response(
    response: UiGatewayResponse,
    platform_csp: &HeaderValue,
) -> Result<Response, UiContentError> {
    if response.body.len() > MAX_UI_RESPONSE_BODY_BYTES {
        return Err(UiContentError::BodyTooLarge);
    }
    if response
        .headers
        .get_all(header::SET_COOKIE)
        .iter()
        .next()
        .is_some()
    {
        return Err(UiContentError::GuestSetCookie);
    }
    if response.headers.contains_key(header::LOCATION)
        || response
            .headers
            .contains_key(HeaderName::from_static("refresh"))
    {
        return Err(UiContentError::GuestRedirect);
    }
    let mut headers = sanitize_guest_response_headers(response.headers);
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    apply_platform_csp(&mut headers, platform_csp);
    let mut result = Response::new(Body::from(response.body));
    *result.status_mut() = response.status;
    *result.headers_mut() = headers;
    Ok(result)
}

pub(super) fn sanitize_guest_request_headers(mut headers: HeaderMap) -> HeaderMap {
    for name in [
        header::AUTHORIZATION,
        header::COOKIE,
        header::CONNECTION,
        header::PROXY_AUTHORIZATION,
        header::PROXY_AUTHENTICATE,
        header::TE,
        header::TRAILER,
        header::TRANSFER_ENCODING,
        header::UPGRADE,
        header::HOST,
        header::ORIGIN,
        header::REFERER,
    ] {
        headers.remove(name);
    }
    for name in ["x-forwarded-for", "x-forwarded-host", "x-forwarded-proto"] {
        headers.remove(name);
    }
    headers
}

pub(super) fn sanitize_guest_response_headers(mut headers: HeaderMap) -> HeaderMap {
    for name in [
        header::SET_COOKIE,
        header::AUTHORIZATION,
        header::PROXY_AUTHENTICATE,
        header::PROXY_AUTHORIZATION,
        header::CONNECTION,
        header::TRANSFER_ENCODING,
        header::CONTENT_SECURITY_POLICY,
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        header::ACCESS_CONTROL_ALLOW_CREDENTIALS,
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        header::ACCESS_CONTROL_ALLOW_METHODS,
        header::ACCESS_CONTROL_EXPOSE_HEADERS,
        header::LOCATION,
        HeaderName::from_static("refresh"),
    ] {
        headers.remove(name);
    }
    headers
}

pub(super) fn apply_platform_csp(headers: &mut HeaderMap, platform_csp: &HeaderValue) {
    headers.insert(header::CONTENT_SECURITY_POLICY, platform_csp.clone());
}

pub(super) fn platform_csp_value(platform_origin: &str) -> Result<HeaderValue, UiContentError> {
    HeaderValue::from_str(&format!(
        "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self'; font-src 'self'; worker-src 'none'; object-src 'none'; frame-src 'none'; frame-ancestors {platform_origin}; connect-src 'self'; form-action 'none'; base-uri 'none'"
    ))
    .map_err(|_| UiContentError::Unavailable)
}

pub(super) fn map_method(method: &http::Method) -> Option<HttpMethod> {
    Some(match method.as_str() {
        "GET" => HttpMethod::Get,
        "HEAD" => HttpMethod::Head,
        "POST" => HttpMethod::Post,
        "PUT" => HttpMethod::Put,
        "PATCH" => HttpMethod::Patch,
        "DELETE" => HttpMethod::Delete,
        "OPTIONS" => HttpMethod::Options,
        _ => return None,
    })
}

pub(super) fn hex_bytes(bytes: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(64);
    for byte in bytes {
        result.push(HEX[usize::from(byte >> 4)] as char);
        result.push(HEX[usize::from(byte & 0x0f)] as char);
    }
    result
}
