//! Authenticated UI gateway authority and route admission contracts.

use async_trait::async_trait;
use http::{HeaderMap, Method};
use uuid::Uuid;

use crate::{Exposure, GatewayEdgeError, GatewayRequest, GatewayResponse, GatewayRouteBinding};

use super::headers::{
    gateway_path_targets_route, reject_ui_set_cookie, strip_ui_guest_headers,
    validate_canonical_path,
};

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
#[async_trait]
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
