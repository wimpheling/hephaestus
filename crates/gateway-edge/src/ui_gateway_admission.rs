//! Boundary for requests arriving from the trusted UI handler.
//!
//! This module deliberately contains no database or credential implementation.
//! The concrete provider belongs in `gateway-postgres` and must resolve the
//! route and gateway revision from the child session, never from UI input.

use crate::{GatewayEdgeError, GatewayRequest, GatewayResponse, GatewayRouteBinding};
use gateway_domain::{Exposure, RoutePath};
use http::{HeaderMap, HeaderName, Method};
use uuid::Uuid;

/// The only request kinds the UI origin may ask the gateway to serve.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiGatewayRequestKind {
    /// A path under a managed release route; exact static-file serving is a
    /// separate UI policy checked before this gateway seam.
    Managed,
    /// An exact API route declared by the release.
    Api,
}

/// Safe, typed authority issued by the authenticated UI handler.
///
/// The handler supplies the child and actor identities and the exact request
/// tuple.  It never supplies a gateway ID, revision ID, bearer token, session
/// secret, or raw parent session identifier.  The provider derives those
/// values from the child row and current release state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiGatewayAuthority {
    /// Canonical child browser session identity.
    pub child_session_id: Uuid,
    /// Authenticated actor bound to the child session.
    pub actor_id: Uuid,
    /// Canonical organization/tenant bound to the child session.
    pub organization_id: Uuid,
    /// Installation selected by the child session.
    pub installation_id: Uuid,
    /// Current installation generation asserted by the UI handler.
    pub generation_id: Uuid,
    /// Canonical request path component. Managed requests are platform-relative
    /// and include the immutable UI route base without a leading slash. API
    /// requests are absolute gateway paths with exactly one leading slash.
    ///
    /// This is intentionally not a gateway path.  The provider maps it to a
    /// selected route after checking the current generation and bindings.
    pub canonical_request_path: String,
    /// Typed route kind declared by the UI verifier.
    pub request_kind: UiGatewayRequestKind,
    /// Exact method asserted by the UI verifier.
    pub method: Method,
}

impl UiGatewayAuthority {
    /// Checks the structural parts that do not require durable authority.
    ///
    /// The provider still rechecks every identity and permission at fresh
    /// database time.  This method only rejects malformed or mismatched input.
    ///
    /// # Errors
    ///
    /// Returns [`GatewayEdgeError::Contract`] when the authority identities,
    /// method, or canonical path do not match the request.
    pub fn validate_for(
        &self,
        method: &Method,
        request_path_and_query: &str,
    ) -> Result<(), GatewayEdgeError> {
        if self.child_session_id.is_nil()
            || self.actor_id.is_nil()
            || self.organization_id.is_nil()
            || self.installation_id.is_nil()
            || self.generation_id.is_nil()
        {
            return Err(GatewayEdgeError::Contract(
                "UI authority has a nil identity",
            ));
        }
        let request_path = request_path_and_query
            .split_once('?')
            .map_or(request_path_and_query, |(path, _)| path);
        if self.method != *method || self.canonical_request_path != request_path {
            return Err(GatewayEdgeError::Contract(
                "UI authority does not match request",
            ));
        }
        validate_canonical_path(self.request_kind, request_path)
    }
}

/// Request envelope received by the trusted UI-origin handler.
///
/// The handler has already authenticated and exchanged the one-time handoff;
/// this envelope carries only the safe result metadata and the browser HTTP
/// request.  Credential-bearing browser headers are stripped before this
/// envelope reaches a guest handler.
#[derive(Clone)]
pub struct UiGatewayRequest {
    /// Safe authority for this one request.
    pub authority: UiGatewayAuthority,
    /// Browser method, checked against `authority.method`.
    pub method: Method,
    /// Canonical request path plus an optional opaque query.
    pub request_path_and_query: String,
    /// Browser headers before UI guest-policy stripping.
    pub headers: HeaderMap,
    /// Bounded request body.
    pub body: bytes::Bytes,
    /// Metadata trusted by the loopback UI-origin handler.
    pub trusted: crate::TrustedRequestMetadata,
}

/// Route and canonical gateway path selected by the durable authority port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiGatewayAdmission {
    /// Current route and immutable released handler revision selected by the
    /// provider.  The caller cannot provide this value.
    pub route: GatewayRouteBinding,
    /// Canonical `/gateway/...` path passed to the existing dispatcher core.
    pub gateway_path_and_query: String,
}

impl UiGatewayAdmission {
    /// Rejects a provider result that cannot enter the authenticated path.
    ///
    /// # Errors
    ///
    /// Returns a contract or route error when the authority, exposure, route,
    /// method, or canonical gateway path is invalid.
    pub fn validate_for(
        &self,
        authority: &UiGatewayAuthority,
        method: &Method,
        request_path_and_query: &str,
    ) -> Result<(), GatewayEdgeError> {
        authority.validate_for(method, request_path_and_query)?;
        if self.route.exposure != Exposure::HephAuthenticated {
            return Err(GatewayEdgeError::Contract(
                "UI admission selected a public or unknown exposure",
            ));
        }
        self.route.validate()?;
        let gateway_path = self
            .gateway_path_and_query
            .split_once('?')
            .map_or(self.gateway_path_and_query.as_str(), |(path, _)| path);
        if !self.route.methods.contains(method)
            || !gateway_path.starts_with('/')
            || gateway_path.contains("//")
            || gateway_path.contains('#')
            || gateway_path.contains('%')
            || !gateway_path_targets_route(&self.route, &self.gateway_path_and_query)
        {
            return Err(GatewayEdgeError::Contract("UI admission route mismatch"));
        }
        Ok(())
    }
}

/// Converts an admitted UI request into the existing dispatcher envelope.
///
/// `GatewayDispatcher::dispatch_ui` should call this immediately before the
/// private admitted invocation helper. No browser path or header is copied
/// into the guest envelope without the structural and route checks above.
///
/// # Errors
///
/// Returns a contract or route error when the request authority or selected
/// route does not match.
pub fn prepare_gateway_request(
    request: &UiGatewayRequest,
    admission: &UiGatewayAdmission,
) -> Result<GatewayRequest, GatewayEdgeError> {
    admission.validate_for(
        &request.authority,
        &request.method,
        &request.request_path_and_query,
    )?;
    let mut headers = request.headers.clone();
    strip_ui_guest_headers(&mut headers);
    Ok(GatewayRequest {
        method: request.method.clone(),
        path_and_query: admission.gateway_path_and_query.clone(),
        headers,
        body: request.body.clone(),
        trusted: request.trusted.clone(),
    })
}

/// Applies the UI response policy before the response reaches the browser.
///
/// # Errors
///
/// Returns [`GatewayEdgeError::Contract`] when the guest attempts to set a
/// browser cookie.
pub fn validate_ui_response(response: &GatewayResponse) -> Result<(), GatewayEdgeError> {
    reject_ui_set_cookie(response)
}

/// Durable child-session authority and route-selection port.
///
/// Implementations must check child, parent, account, installation, generation,
/// source release, and current binding permissions at fresh DB time. They must
/// derive the gateway route/revision and return a safe denial without exposing
/// whether a secret, parent, or permission failed. The recorder's `accepted_ui`
/// call remains a second required transaction boundary: it rechecks this safe
/// authority and exact route while inserting the invocation row.
#[async_trait::async_trait]
pub trait UiGatewayAdmissionProvider: Send + Sync {
    /// Selects the current exact route for a trusted UI request.
    ///
    /// # Errors
    ///
    /// Returns a redacted admission failure when durable authority or route
    /// resolution cannot authorize the request.
    async fn admit(
        &self,
        request: &UiGatewayRequest,
    ) -> Result<UiGatewayAdmission, UiGatewayAdmissionError>;
}

/// Redacted result classes for the UI-origin handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiGatewayAdmissionError {
    /// The child, actor, generation, route, or permission is not authorized.
    Denied,
    /// The requested UI route does not exist in the current release.
    NotFound,
    /// Durable authority or gateway execution state is unavailable.
    Unavailable,
}

/// Maps a UI authority failure to the safe browser response.
#[must_use]
pub fn admission_failure_response(error: UiGatewayAdmissionError) -> GatewayResponse {
    let status = match error {
        UiGatewayAdmissionError::Denied => http::StatusCode::UNAUTHORIZED,
        UiGatewayAdmissionError::NotFound => http::StatusCode::NOT_FOUND,
        UiGatewayAdmissionError::Unavailable => http::StatusCode::SERVICE_UNAVAILABLE,
    };
    GatewayResponse {
        status,
        headers: HeaderMap::new(),
        body: bytes::Bytes::new(),
        mailbox_publication: None,
    }
}

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

fn validate_canonical_path(kind: UiGatewayRequestKind, path: &str) -> Result<(), GatewayEdgeError> {
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

fn gateway_path_targets_route(route: &GatewayRouteBinding, path_and_query: &str) -> bool {
    let path = path_and_query
        .split_once('?')
        .map_or(path_and_query, |(path, _)| path);
    let route_path = route.public_path();
    path == route_path
        || path
            .strip_prefix(&route_path)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::GatewayLimits;
    use bytes::Bytes;
    use http::{HeaderValue, StatusCode};
    use std::{collections::BTreeSet, time::Duration};

    fn authority(method: Method, path: &str) -> UiGatewayAuthority {
        UiGatewayAuthority {
            child_session_id: Uuid::new_v4(),
            actor_id: Uuid::new_v4(),
            organization_id: Uuid::new_v4(),
            installation_id: Uuid::new_v4(),
            generation_id: Uuid::new_v4(),
            canonical_request_path: path.to_owned(),
            request_kind: UiGatewayRequestKind::Managed,
            method,
        }
    }

    fn route(exposure: Exposure) -> GatewayRouteBinding {
        GatewayRouteBinding {
            route_id: Uuid::new_v4(),
            gateway_revision_id: Uuid::new_v4(),
            exposure,
            path_prefix: "ui/release".to_owned(),
            methods: BTreeSet::from([Method::GET]),
            limits: GatewayLimits {
                max_request_body_bytes: 1024,
                max_response_body_bytes: 1024,
                max_request_headers: 32,
                max_response_headers: 32,
                max_path_and_query_bytes: 1024,
                execution_timeout: Duration::from_secs(1),
            },
        }
    }

    #[test]
    fn managed_and_api_paths_have_distinct_canonical_grammars() {
        let managed = authority(Method::GET, "ui/release/index.html");
        assert!(
            managed
                .validate_for(&Method::GET, "ui/release/index.html?next=%2F//opaque")
                .is_ok()
        );
        assert!(
            managed
                .validate_for(&Method::GET, "/ui/release/index.html")
                .is_err()
        );
        assert!(
            managed
                .validate_for(&Method::GET, "ui/release/../secret")
                .is_err()
        );

        let api = UiGatewayAuthority {
            request_kind: UiGatewayRequestKind::Api,
            canonical_request_path: "/api/v1/items".to_owned(),
            ..managed
        };
        assert!(
            api.validate_for(&Method::GET, "/api/v1/items?next=%2F//opaque")
                .is_ok()
        );
        assert!(api.validate_for(&Method::GET, "api/v1/items").is_err());
        assert!(
            api.validate_for(&Method::GET, "/api/v1/%2e%2e/secret")
                .is_err()
        );
    }

    #[test]
    fn admission_preserves_opaque_query_and_rejects_route_mismatch() {
        let authority = authority(Method::GET, "ui/release/index.html");
        let admission = UiGatewayAdmission {
            route: route(Exposure::HephAuthenticated),
            gateway_path_and_query: "/gateway/ui/release/index.html?next=%2F//opaque".to_owned(),
        };
        assert!(
            admission
                .validate_for(
                    &authority,
                    &Method::GET,
                    "ui/release/index.html?next=%2F//opaque",
                )
                .is_ok()
        );
        let mismatch = UiGatewayAdmission {
            gateway_path_and_query: "/gateway/another-release/index.html".to_owned(),
            ..admission
        };
        assert!(
            mismatch
                .validate_for(&authority, &Method::GET, "ui/release/index.html")
                .is_err()
        );
    }

    #[test]
    fn public_routes_and_invalid_authority_are_rejected() {
        let authority = authority(Method::GET, "ui/release/index.html");
        let public = UiGatewayAdmission {
            route: route(Exposure::Public),
            gateway_path_and_query: "/gateway/ui/release/index.html".to_owned(),
        };
        assert!(
            public
                .validate_for(&authority, &Method::GET, "ui/release/index.html")
                .is_err()
        );
        let mut nil = authority.clone();
        nil.child_session_id = Uuid::nil();
        assert!(
            nil.validate_for(&Method::GET, "ui/release/index.html")
                .is_err()
        );
    }

    #[test]
    fn guest_headers_and_set_cookie_are_rejected() {
        let mut headers = HeaderMap::new();
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
            "x-forwarded-port",
            "x-forwarded-prefix",
            "x-forwarded-server",
        ] {
            headers.insert(name, HeaderValue::from_static("secret"));
        }
        strip_ui_guest_headers(&mut headers);
        assert!(headers.is_empty());

        let mut response = GatewayResponse {
            status: StatusCode::OK,
            headers: HeaderMap::new(),
            body: Bytes::new(),
            mailbox_publication: None,
        };
        response
            .headers
            .insert("set-cookie", HeaderValue::from_static("sid=secret"));
        assert!(reject_ui_set_cookie(&response).is_err());
    }

    #[test]
    fn admission_errors_have_safe_statuses() {
        assert_eq!(
            admission_failure_response(UiGatewayAdmissionError::Denied).status,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            admission_failure_response(UiGatewayAdmissionError::NotFound).status,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            admission_failure_response(UiGatewayAdmissionError::Unavailable).status,
            StatusCode::SERVICE_UNAVAILABLE
        );
    }
}
