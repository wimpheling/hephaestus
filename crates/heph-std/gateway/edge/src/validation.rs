use bytes::Bytes;
use http::{HeaderMap, HeaderName, StatusCode};
use vm_trait::{PrivateHttpResponse, PrivateMailboxPublication};

use crate::{
    Exposure, GatewayEdgeError, GatewayRequest, GatewayResponse, GatewayRouteBinding,
    UNTRUSTED_FORWARDING_HEADERS,
};

pub fn validate_request(
    route: &GatewayRouteBinding,
    request: &GatewayRequest,
) -> Result<(), GatewayEdgeError> {
    validate_request_for_exposure(route, request, Exposure::Public)
}

pub fn validate_ui_request(
    route: &GatewayRouteBinding,
    request: &GatewayRequest,
) -> Result<(), GatewayEdgeError> {
    validate_request_for_exposure(route, request, Exposure::HephAuthenticated)
}

fn validate_request_for_exposure(
    route: &GatewayRouteBinding,
    request: &GatewayRequest,
    expected_exposure: Exposure,
) -> Result<(), GatewayEdgeError> {
    if route.exposure != expected_exposure {
        return Err(GatewayEdgeError::Contract(
            "gateway exposure does not match dispatcher boundary",
        ));
    }
    if !request.path_and_query.starts_with('/')
        || request.path_and_query.contains("//")
        || request
            .path_and_query
            .split('?')
            .next()
            .is_some_and(|path| path.split('/').any(|part| part == "." || part == ".."))
    {
        return Err(GatewayEdgeError::Contract("ambiguous request path"));
    }
    if !request_targets_route(route, &request.path_and_query) {
        return Err(GatewayEdgeError::Contract("route mismatch"));
    }
    if !route.methods.contains(&request.method) {
        return Err(GatewayEdgeError::Contract("method is not allowed"));
    }
    if request.path_and_query.len() > route.limits.max_path_and_query_bytes
        || request.body.len() > route.limits.max_request_body_bytes
        || request.headers.len() > route.limits.max_request_headers
    {
        return Err(GatewayEdgeError::Contract("request exceeds route limits"));
    }
    if request.headers.keys().any(|name| {
        is_hop_by_hop(name)
            || UNTRUSTED_FORWARDING_HEADERS
                .iter()
                .any(|blocked| name.as_str().eq_ignore_ascii_case(blocked))
    }) {
        return Err(GatewayEdgeError::Contract("forbidden request header"));
    }
    Ok(())
}

fn request_targets_route(route: &GatewayRouteBinding, path_and_query: &str) -> bool {
    let path = path_and_query
        .split_once('?')
        .map_or(path_and_query, |(path, _)| path);
    let public_path = route.public_path();
    path == public_path
        || path
            .strip_prefix(&public_path)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

pub fn validate_response(
    route: &GatewayRouteBinding,
    response: &GatewayResponse,
) -> Result<(), GatewayEdgeError> {
    if response.body.len() > route.limits.max_response_body_bytes
        || response.headers.len() > route.limits.max_response_headers
        || response.headers.keys().any(is_hop_by_hop)
    {
        return Err(GatewayEdgeError::Contract("response violates route limits"));
    }
    if let Some(publication) = &response.mailbox_publication {
        validate_mailbox_publication(publication)?;
    }
    Ok(())
}

pub fn validate_mailbox_publication(
    publication: &PrivateMailboxPublication,
) -> Result<(), GatewayEdgeError> {
    const MAX_BODY_BYTES: usize = 1_048_576;
    const MAX_HEADERS: usize = 32;
    const MAX_SLOT_BYTES: usize = 64;
    const MAX_METHOD_BYTES: usize = 16;
    const MAX_ROUTE_BYTES: usize = 1_024;
    const MAX_HEADER_NAME_BYTES: usize = 64;
    const MAX_HEADER_VALUE_BYTES: usize = 1_024;
    const MAX_CONTENT_TYPE_BYTES: usize = 256;
    const MAX_TRACE_CONTEXT_BYTES: usize = 512;
    const MAX_DEDUPLICATION_KEY_BYTES: usize = 256;
    let bounded =
        |value: &str, maximum| !value.is_empty() && value.len() <= maximum && !value.contains('\0');
    let valid_optional = |value: &Option<String>, maximum| {
        value
            .as_ref()
            .is_none_or(|value| value.len() <= maximum && !value.contains('\0'))
    };
    if !bounded(&publication.slot, MAX_SLOT_BYTES)
        || publication.method.is_empty()
        || publication.method.len() > MAX_METHOD_BYTES
        || !publication
            .method
            .bytes()
            .all(|byte| byte.is_ascii_uppercase())
        || !bounded(&publication.route, MAX_ROUTE_BYTES)
        || !publication.route.starts_with('/')
        || publication.headers.len() > MAX_HEADERS
        || publication.headers.iter().any(|(name, value)| {
            !bounded(name, MAX_HEADER_NAME_BYTES)
                || value.len() > MAX_HEADER_VALUE_BYTES
                || value.contains('\0')
        })
        || !valid_optional(&publication.content_type, MAX_CONTENT_TYPE_BYTES)
        || !valid_optional(&publication.trace_context, MAX_TRACE_CONTEXT_BYTES)
        || publication.body.len() > MAX_BODY_BYTES
        || !bounded(&publication.deduplication_key, MAX_DEDUPLICATION_KEY_BYTES)
    {
        return Err(GatewayEdgeError::Contract(
            "mailbox publication violates gateway limits",
        ));
    }
    Ok(())
}

fn is_hop_by_hop(name: &HeaderName) -> bool {
    matches!(
        name.as_str(),
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
    )
}

pub fn empty_response(status: StatusCode) -> GatewayResponse {
    GatewayResponse {
        status,
        headers: HeaderMap::new(),
        body: Bytes::new(),
        mailbox_publication: None,
    }
}

pub fn gateway_response(response: PrivateHttpResponse) -> GatewayResponse {
    GatewayResponse {
        status: response.status,
        headers: response.headers,
        body: response.body,
        mailbox_publication: response.mailbox_publication,
    }
}
