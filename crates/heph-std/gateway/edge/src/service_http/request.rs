use bytes::Bytes;
use http::{
    HeaderMap, HeaderName, HeaderValue, Request, Uri,
    header::{CONNECTION, CONTENT_LENGTH, HOST},
};
use http_body_util::Full;
use std::collections::HashSet;

use super::policy::ServiceHttpPolicy;
use crate::{GatewayEdgeError, GatewayRequest};

pub(super) fn canonical_request(
    request: GatewayRequest,
    policy: ServiceHttpPolicy,
) -> Result<Request<Full<Bytes>>, GatewayEdgeError> {
    if policy.exchange_timeout.is_zero()
        || request.path_and_query.len() > policy.max_path_and_query_bytes
        || request.body.len() > policy.max_request_body_bytes
        || request.headers.len() > policy.max_request_headers
        || !is_canonical_origin_form(&request.path_and_query)
    {
        return Err(GatewayEdgeError::Contract(
            "service request exceeds canonical HTTP bounds",
        ));
    }
    let nominated = connection_nominated_headers(&request.headers)?;
    let mut headers = request.headers;
    let names = headers.keys().cloned().collect::<Vec<_>>();
    for name in names {
        if is_forbidden_request_header(&name)
            || nominated.contains(&name)
            || name == HOST
            || name == CONTENT_LENGTH
        {
            headers.remove(name);
        }
    }
    let host = HeaderValue::try_from(request.trusted.authority.as_str())
        .map_err(|_| GatewayEdgeError::Contract("invalid trusted service authority"))?;
    if host.is_empty() {
        return Err(GatewayEdgeError::Contract(
            "trusted service authority is empty",
        ));
    }
    headers.insert(HOST, host);
    headers.insert(
        CONTENT_LENGTH,
        HeaderValue::from_str(&request.body.len().to_string())
            .map_err(|_| GatewayEdgeError::Contract("invalid service request length"))?,
    );
    headers.insert(CONNECTION, HeaderValue::from_static("close"));
    if serialized_header_bytes(&headers) > policy.max_wire_header_bytes {
        return Err(GatewayEdgeError::Contract(
            "service request headers exceed the byte limit",
        ));
    }

    let uri = request
        .path_and_query
        .parse::<Uri>()
        .map_err(|_| GatewayEdgeError::Contract("invalid service request target"))?;
    let mut outbound = Request::new(Full::new(request.body));
    *outbound.method_mut() = request.method;
    *outbound.uri_mut() = uri;
    *outbound.headers_mut() = headers;
    Ok(outbound)
}

fn is_canonical_origin_form(value: &str) -> bool {
    let (path, _) = value.split_once('?').map_or((value, ""), |parts| parts);
    value.starts_with('/')
        && !value.contains('#')
        && !path.contains('%')
        && !path.contains("//")
        && !path.split('/').any(|part| part == "." || part == "..")
        && !value
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
}

pub(super) fn connection_nominated_headers(
    headers: &HeaderMap,
) -> Result<HashSet<HeaderName>, GatewayEdgeError> {
    let mut nominated = HashSet::new();
    for value in &headers.get_all(CONNECTION) {
        let value = value
            .to_str()
            .map_err(|_| GatewayEdgeError::Contract("invalid Connection header"))?;
        for token in value.split(',').map(str::trim) {
            if token.is_empty() {
                return Err(GatewayEdgeError::Contract("invalid Connection header"));
            }
            let name = HeaderName::from_bytes(token.as_bytes())
                .map_err(|_| GatewayEdgeError::Contract("invalid Connection header"))?;
            nominated.insert(name);
        }
    }
    Ok(nominated)
}

fn is_forbidden_request_header(name: &HeaderName) -> bool {
    matches!(
        name.as_str(),
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "proxy-connection"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
            | "expect"
            | "forwarded"
            | "x-forwarded-for"
            | "x-forwarded-host"
            | "x-forwarded-proto"
    )
}

pub(super) fn serialized_header_bytes(headers: &HeaderMap) -> usize {
    headers
        .iter()
        .map(|(name, value)| name.as_str().len() + value.as_bytes().len() + 4)
        .sum()
}
