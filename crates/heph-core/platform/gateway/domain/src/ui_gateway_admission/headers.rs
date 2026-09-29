//! UI-origin header stripping and canonical path checks.

use http::{HeaderMap, HeaderName};

use crate::{GatewayEdgeError, GatewayResponse, GatewayRouteBinding, RoutePath};

use super::admission::UiGatewayRequestKind;

/// Removes browser credentials and edge-routing headers before guest code.
pub fn strip_ui_guest_headers(headers: &mut HeaderMap) {
    for name in [
        "authorization",
        "cookie",
        "host",
        "proxy-authorization",
        "x-api-key",
        "x-auth-token",
        "x-access-token",
        "forwarded",
        "x-forwarded-for",
        "x-forwarded-host",
        "x-forwarded-proto",
    ] {
        // These are all static valid header names; using `remove` keeps the
        // operation idempotent for adapters that already stripped a subset.
        headers.remove(HeaderName::from_static(name));
    }
    let forwarding_variants = headers
        .keys()
        .filter(|name| name.as_str().starts_with("x-forwarded-"))
        .cloned()
        .collect::<Vec<_>>();
    for name in forwarding_variants {
        headers.remove(name);
    }
}

/// Rejects guest responses that could create a browser credential.
///
/// # Errors
///
/// Returns [`GatewayEdgeError::Contract`] when the response contains a
/// `Set-Cookie` header.
pub fn reject_ui_set_cookie(response: &GatewayResponse) -> Result<(), GatewayEdgeError> {
    if response.headers.contains_key("set-cookie") {
        return Err(GatewayEdgeError::Contract(
            "UI guest responses cannot set cookies",
        ));
    }
    Ok(())
}

pub(super) fn validate_canonical_path(
    kind: UiGatewayRequestKind,
    path: &str,
) -> Result<(), GatewayEdgeError> {
    if path.chars().any(char::is_whitespace)
        || path.contains(['?', '#', '%', '\\'])
        || path.contains("//")
    {
        return Err(GatewayEdgeError::Contract("ambiguous UI request path"));
    }
    match kind {
        UiGatewayRequestKind::Managed => {
            if path.is_empty()
                || path.starts_with('/')
                || path
                    .split('/')
                    .any(|segment| segment.is_empty() || segment == "." || segment == "..")
            {
                return Err(GatewayEdgeError::Contract(
                    "managed UI path is not platform-relative",
                ));
            }
        }
        UiGatewayRequestKind::Api => {
            if RoutePath::parse(path.to_owned()).is_err() {
                return Err(GatewayEdgeError::Contract(
                    "API UI path is not a canonical gateway route",
                ));
            }
        }
    }
    Ok(())
}

pub(super) fn gateway_path_targets_route(
    route: &GatewayRouteBinding,
    path_and_query: &str,
) -> bool {
    let path = path_and_query
        .split_once('?')
        .map_or(path_and_query, |(path, _)| path);
    let route_path = route.public_path();
    path == route_path
        || path
            .strip_prefix(&route_path)
            .is_some_and(|suffix| suffix.starts_with('/'))
}
