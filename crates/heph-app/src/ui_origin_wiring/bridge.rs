//! Bridge from the authenticated UI content handler to the gateway VM path.

use async_trait::async_trait;
use bytes::Bytes;
use gateway_edge::{
    GatewayDispatcher, GatewayScheme, TrustedRequestMetadata, UiDispatchResult,
    UiGatewayAdmissionProvider, UiGatewayAuthority, UiGatewayRequest, UiGatewayRequestKind,
};
use http::{HeaderMap, Method};
use release_service::{
    UiGatewayRequestKind as ReleaseGatewayRequestKind, UiGenerationHost, UiNamespace, UiPublicPort,
};
use std::{
    net::{IpAddr, Ipv4Addr},
    sync::Arc,
};

use crate::ui_browser_content::{
    UiContentError, UiGatewayDispatchAuthority, UiGatewayDispatchResult, UiGatewayDispatcher,
    UiGatewayResponse,
};

/// Object-safe call surface retaining the concrete dispatcher required by
/// `GatewayDispatcher::dispatch_ui`.
#[async_trait]
pub trait UiDispatchCore: Send + Sync {
    /// Dispatches through the existing VM handler and invocation lifecycle,
    /// retaining the typed admission/execution disposition for audit.
    async fn dispatch_ui_request_detailed(
        &self,
        request: UiGatewayRequest,
        provider: &dyn UiGatewayAdmissionProvider,
    ) -> UiDispatchResult;
}

#[async_trait]
impl<R, H, I> UiDispatchCore for GatewayDispatcher<R, H, I>
where
    R: gateway_edge::GatewayRouteResolver + Send + Sync,
    H: gateway_edge::GatewayVmHandler + Send + Sync,
    I: gateway_edge::GatewayInvocationRecorder + Send + Sync,
{
    async fn dispatch_ui_request_detailed(
        &self,
        request: UiGatewayRequest,
        provider: &dyn UiGatewayAdmissionProvider,
    ) -> UiDispatchResult {
        self.dispatch_ui_detailed(request, provider).await
    }
}

/// Bridge from the authenticated content handler to the real gateway VM path.
pub struct RealUiGatewayDispatcher<C, P> {
    core: Arc<C>,
    admission: Arc<P>,
    namespace: UiNamespace,
    public_port: UiPublicPort,
}

impl<C, P> RealUiGatewayDispatcher<C, P> {
    /// Creates a bridge over the real dispatcher and worker authority.
    #[must_use]
    pub const fn new(
        core: Arc<C>,
        admission: Arc<P>,
        namespace: UiNamespace,
        public_port: UiPublicPort,
    ) -> Self {
        Self {
            core,
            admission,
            namespace,
            public_port,
        }
    }
}

#[async_trait]
impl<C, P> UiGatewayDispatcher for RealUiGatewayDispatcher<C, P>
where
    C: UiDispatchCore + 'static,
    P: UiGatewayAdmissionProvider + 'static,
{
    async fn dispatch_ui_detailed(
        &self,
        authority: UiGatewayDispatchAuthority,
        method: gateway_domain::HttpMethod,
        path_and_query: String,
        headers: HeaderMap,
        body: Bytes,
    ) -> Result<UiGatewayDispatchResult, UiContentError> {
        let (request_kind, canonical_path, request_path_and_query) = match authority.request.kind {
            ReleaseGatewayRequestKind::Managed => {
                let canonical_path = authority
                    .request
                    .path
                    .as_str()
                    .strip_prefix('/')
                    .ok_or(UiContentError::InvalidRequest)?
                    .to_owned();
                let request_path_and_query = strip_managed_leading_slash(&path_and_query)?;
                (
                    UiGatewayRequestKind::Managed,
                    canonical_path,
                    request_path_and_query,
                )
            }
            ReleaseGatewayRequestKind::Api => (
                UiGatewayRequestKind::Api,
                authority.request.path.as_str().to_owned(),
                path_and_query,
            ),
        };
        let method = to_http_method(method);
        let request = UiGatewayRequest {
            authority: UiGatewayAuthority {
                child_session_id: authority.child_session_id.as_uuid(),
                actor_id: authority.actor_id.as_uuid(),
                organization_id: authority.organization_id.as_uuid(),
                installation_id: authority.installation_id.as_uuid(),
                generation_id: authority.generation_id.as_uuid(),
                canonical_request_path: canonical_path,
                request_kind,
                method: method.clone(),
            },
            method: method.clone(),
            request_path_and_query,
            headers,
            body,
            trusted: TrustedRequestMetadata {
                scheme: GatewayScheme::Https,
                authority: UiGenerationHost::from_generation_id(authority.generation_id)
                    .authority(&self.namespace, self.public_port),
                client_address: IpAddr::V4(Ipv4Addr::LOCALHOST),
                request_id: authority.request_id.as_uuid(),
            },
        };
        let dispatch = self
            .core
            .dispatch_ui_request_detailed(request, self.admission.as_ref())
            .await;
        Ok(UiGatewayDispatchResult {
            response: UiGatewayResponse {
                status: dispatch.response.response.status,
                headers: dispatch.response.response.headers.clone(),
                body: dispatch.response.response.body.clone(),
            },
            disposition: dispatch.disposition,
        })
    }
}

fn strip_managed_leading_slash(path_and_query: &str) -> Result<String, UiContentError> {
    path_and_query
        .strip_prefix('/')
        .map(str::to_owned)
        .ok_or(UiContentError::InvalidRequest)
}

const fn to_http_method(method: gateway_domain::HttpMethod) -> Method {
    match method {
        gateway_domain::HttpMethod::Get => Method::GET,
        gateway_domain::HttpMethod::Head => Method::HEAD,
        gateway_domain::HttpMethod::Post => Method::POST,
        gateway_domain::HttpMethod::Put => Method::PUT,
        gateway_domain::HttpMethod::Patch => Method::PATCH,
        gateway_domain::HttpMethod::Delete => Method::DELETE,
        gateway_domain::HttpMethod::Options => Method::OPTIONS,
    }
}
