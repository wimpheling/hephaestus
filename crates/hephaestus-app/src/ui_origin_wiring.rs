//! Composition seams for the dedicated loopback UI-origin listener.

use async_trait::async_trait;
use axum::{Router, middleware, middleware::Next};
use bytes::Bytes;
use gateway_edge::{
    GatewayDispatcher, GatewayProviderResponse, GatewayScheme, TrustedRequestMetadata,
    UiGatewayAdmissionProvider, UiGatewayAuthority, UiGatewayRequest, UiGatewayRequestKind,
};
use http::{HeaderMap, Method, StatusCode};
use release_service::{
    UiGatewayRequestKind as ReleaseGatewayRequestKind, UiGenerationHost, UiNamespace, UiPublicPort,
};
use std::{
    net::{IpAddr, Ipv4Addr},
    sync::Arc,
    time::Duration,
};

use crate::ui_browser_content::{
    UiContentError, UiGatewayDispatchAuthority, UiGatewayDispatcher, UiGatewayResponse,
};

/// Object-safe call surface retaining the concrete dispatcher required by
/// `GatewayDispatcher::dispatch_ui`.
#[async_trait]
pub trait UiDispatchCore: Send + Sync {
    /// Dispatches through the existing VM handler and invocation lifecycle.
    async fn dispatch_ui_request(
        &self,
        request: UiGatewayRequest,
        provider: &dyn UiGatewayAdmissionProvider,
    ) -> GatewayProviderResponse;
}

#[async_trait]
impl<R, H, I> UiDispatchCore for GatewayDispatcher<R, H, I>
where
    R: gateway_edge::GatewayRouteResolver + Send + Sync,
    H: gateway_edge::GatewayVmHandler + Send + Sync,
    I: gateway_edge::GatewayInvocationRecorder + Send + Sync,
{
    async fn dispatch_ui_request(
        &self,
        request: UiGatewayRequest,
        provider: &dyn UiGatewayAdmissionProvider,
    ) -> GatewayProviderResponse {
        self.dispatch_ui(request, provider).await
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
    async fn dispatch_ui(
        &self,
        authority: UiGatewayDispatchAuthority,
        method: gateway_domain::HttpMethod,
        path_and_query: String,
        headers: HeaderMap,
        body: Bytes,
    ) -> Result<UiGatewayResponse, UiContentError> {
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
        let response = self
            .core
            .dispatch_ui_request(request, self.admission.as_ref())
            .await;
        Ok(UiGatewayResponse {
            status: response.response.status,
            headers: response.response.headers,
            body: response.response.body,
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

/// Applies one bounded deadline and permit pool to every UI-origin route.
pub fn bounded_ui_router<S>(
    router: Router<S>,
    permits: Arc<tokio::sync::Semaphore>,
    deadline: Duration,
) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    router.layer(middleware::from_fn(
        move |request: http::Request<axum::body::Body>, next: Next| {
            let permits = Arc::clone(&permits);
            async move {
                let Ok(_permit) = permits.try_acquire_owned() else {
                    return axum::response::Response::builder()
                        .status(StatusCode::TOO_MANY_REQUESTS)
                        .body(axum::body::Body::empty())
                        .expect("bounded UI response");
                };
                tokio::time::timeout(deadline, next.run(request))
                    .await
                    .unwrap_or_else(|_| {
                        axum::response::Response::builder()
                            .status(StatusCode::SERVICE_UNAVAILABLE)
                            .body(axum::body::Body::empty())
                            .expect("bounded UI response")
                    })
            }
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use axum::{body::Body, routing::get};
    use forge_domain::OrganizationId;
    use gateway_edge::{GatewayResponse, UiGatewayAdmission, UiGatewayAdmissionError};
    use identity_domain::{RequestId, UserId};
    use release_domain::{
        UiInstallationGenerationId, UiInstallationId, ui_browser::UiBrowserSessionId,
    };
    use release_service::{UiBrowserHttpPath, UiGatewayRequestProjection};
    use std::sync::Mutex;
    use tower::ServiceExt;
    use uuid::Uuid;

    struct CapturingCore {
        request: Mutex<Option<UiGatewayRequest>>,
    }

    #[async_trait]
    impl UiDispatchCore for CapturingCore {
        async fn dispatch_ui_request(
            &self,
            request: UiGatewayRequest,
            _provider: &dyn UiGatewayAdmissionProvider,
        ) -> GatewayProviderResponse {
            *self.request.lock().expect("capture lock") = Some(request);
            GatewayProviderResponse {
                response: GatewayResponse {
                    status: StatusCode::OK,
                    headers: HeaderMap::new(),
                    body: Bytes::from_static(b"ok"),
                    mailbox_publication: None,
                },
                invocation_id: Uuid::new_v4(),
            }
        }
    }

    struct UnusedAdmission;

    #[async_trait]
    impl UiGatewayAdmissionProvider for UnusedAdmission {
        async fn admit(
            &self,
            _request: &UiGatewayRequest,
        ) -> Result<UiGatewayAdmission, UiGatewayAdmissionError> {
            Err(UiGatewayAdmissionError::Denied)
        }
    }

    fn authority(
        request_id: RequestId,
        kind: ReleaseGatewayRequestKind,
        path: &str,
        method: gateway_domain::HttpMethod,
    ) -> UiGatewayDispatchAuthority {
        UiGatewayDispatchAuthority {
            request_id,
            child_session_id: UiBrowserSessionId::from_uuid(Uuid::new_v4()),
            actor_id: UserId::from_uuid(Uuid::new_v4()),
            organization_id: OrganizationId::from_uuid(Uuid::new_v4()),
            installation_id: UiInstallationId::from_uuid(Uuid::new_v4()),
            generation_id: UiInstallationGenerationId::from_uuid(Uuid::new_v4()),
            request: UiGatewayRequestProjection {
                kind,
                path: UiBrowserHttpPath::parse(path).expect("canonical path"),
                method,
            },
        }
    }

    #[tokio::test]
    async fn bridge_converts_managed_authority_and_preserves_query_and_request_id() {
        let request_id = RequestId::new();
        let core = Arc::new(CapturingCore {
            request: Mutex::new(None),
        });
        let bridge = RealUiGatewayDispatcher::new(
            Arc::clone(&core),
            Arc::new(UnusedAdmission),
            UiNamespace::parse("ui.example.test").expect("namespace"),
            UiPublicPort::https_default(),
        );
        let authority = authority(
            request_id,
            ReleaseGatewayRequestKind::Managed,
            "/reference/assets/app.js",
            gateway_domain::HttpMethod::Get,
        );
        bridge
            .dispatch_ui(
                authority,
                gateway_domain::HttpMethod::Get,
                "/reference/assets/app.js?x=%2F%2F&empty=".to_owned(),
                HeaderMap::new(),
                Bytes::new(),
            )
            .await
            .expect("managed bridge");
        let request = core
            .request
            .lock()
            .expect("capture lock")
            .take()
            .expect("request");
        assert_eq!(
            request.authority.request_kind,
            UiGatewayRequestKind::Managed
        );
        assert_eq!(
            request.authority.canonical_request_path,
            "reference/assets/app.js"
        );
        assert_eq!(
            request.request_path_and_query,
            "reference/assets/app.js?x=%2F%2F&empty="
        );
        assert_eq!(request.trusted.request_id, request_id.as_uuid());
    }

    #[tokio::test]
    async fn bridge_keeps_api_absolute_path_and_empty_query() {
        let core = Arc::new(CapturingCore {
            request: Mutex::new(None),
        });
        let bridge = RealUiGatewayDispatcher::new(
            Arc::clone(&core),
            Arc::new(UnusedAdmission),
            UiNamespace::parse("ui.example.test").expect("namespace"),
            UiPublicPort::https_default(),
        );
        bridge
            .dispatch_ui(
                authority(
                    RequestId::new(),
                    ReleaseGatewayRequestKind::Api,
                    "/api/ready",
                    gateway_domain::HttpMethod::Get,
                ),
                gateway_domain::HttpMethod::Get,
                "/api/ready?".to_owned(),
                HeaderMap::new(),
                Bytes::new(),
            )
            .await
            .expect("API bridge");
        let request = core
            .request
            .lock()
            .expect("capture lock")
            .take()
            .expect("request");
        assert_eq!(request.authority.request_kind, UiGatewayRequestKind::Api);
        assert_eq!(request.authority.canonical_request_path, "/api/ready");
        assert_eq!(request.request_path_and_query, "/api/ready?");
    }

    #[tokio::test]
    async fn bounded_router_enforces_permits_and_deadline() {
        let permits = Arc::new(tokio::sync::Semaphore::new(1));
        let held = permits.clone().try_acquire_owned().expect("permit");
        let busy = bounded_ui_router(
            Router::new().route("/healthz", get(|| async { "ok" })),
            permits.clone(),
            Duration::from_secs(1),
        );
        let response = busy
            .clone()
            .oneshot(
                axum::http::Request::get("/healthz")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("busy response");
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        drop(held);
        let response = busy
            .oneshot(
                axum::http::Request::get("/healthz")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("available response");
        assert_eq!(response.status(), StatusCode::OK);

        let slow = bounded_ui_router(
            Router::new().route(
                "/healthz",
                get(|| async {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                    "late"
                }),
            ),
            Arc::new(tokio::sync::Semaphore::new(1)),
            Duration::from_millis(1),
        );
        let response = slow
            .oneshot(
                axum::http::Request::get("/healthz")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("deadline response");
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }
}
