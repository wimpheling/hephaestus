//! Authenticated generic target discovery for installed release UIs.
//!
//! This route returns only the repository target selected by a live,
//! repository-scoped installation. It never accepts target identity from the
//! browser and never exposes the child cookie or any grant metadata.

use axum::{
    Router,
    body::Body,
    extract::{ConnectInfo, State},
    http::{Request, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use identity_domain::RequestId;
use release_service::{
    ActiveUiGenerationHost, UiBrowserTargetContextProjection, UiGenerationHostResolver,
    UiHostLookupError, UiNamespace, UiPublicPort, UiTargetContextError,
    ui_browser_host::UiGenerationHost,
};
use serde::Serialize;
use std::{net::SocketAddr, sync::Arc, time::Duration};

use crate::{
    ui_audit::{UiAuditRecorder, correlation_id, verified_context},
    ui_repository_git::{
        error_response, parse_child_cookie, request_authority, require_same_origin,
    },
};

const CONTEXT_PATH: &str = "/_heph/ui-context";
const CONTEXT_DEADLINE: Duration = Duration::from_secs(15);

/// State for the authenticated generic installed-UI context route.
pub struct UiContextState {
    host_resolver: Arc<dyn UiGenerationHostResolver>,
    projection: Arc<dyn UiBrowserTargetContextProjection>,
    namespace: UiNamespace,
    public_port: UiPublicPort,
    audit: UiAuditRecorder,
    deadline: Duration,
}

impl UiContextState {
    /// Builds the context route over the existing host and child verifier.
    pub fn new(
        host_resolver: Arc<dyn UiGenerationHostResolver>,
        projection: Arc<dyn UiBrowserTargetContextProjection>,
        namespace: UiNamespace,
        public_port: UiPublicPort,
        audit_sink: Arc<dyn release_service::UiRequestAuditSink>,
    ) -> Self {
        Self {
            host_resolver,
            projection,
            namespace,
            public_port,
            audit: UiAuditRecorder::new(audit_sink),
            deadline: CONTEXT_DEADLINE,
        }
    }
}

/// Registers only the exact generic context path.
pub fn router(state: Arc<UiContextState>) -> Router {
    Router::new()
        .route(CONTEXT_PATH, get(get_context))
        .with_state(state)
}

#[derive(Debug, Serialize)]
struct ContextResponse {
    repository_id: uuid::Uuid,
}

async fn get_context(
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    State(state): State<Arc<UiContextState>>,
    request: Request<Body>,
) -> Response<Body> {
    let request_id = correlation_id(request.extensions());
    if !peer.ip().is_loopback() {
        return deny(&state, request_id, StatusCode::FORBIDDEN, "ui_forbidden").await;
    }
    match tokio::time::timeout(state.deadline, project(&state, request_id, request)).await {
        Ok(Ok(response)) => response,
        Ok(Err((status, code))) => deny(&state, request_id, status, code).await,
        Err(_) => {
            state
                .audit
                .undetermined(
                    request_id,
                    release_service::UiRequestAuditSurface::Content,
                    release_service::UiRequestAuditContext::anonymous(),
                    error_response(StatusCode::SERVICE_UNAVAILABLE, "ui_unavailable"),
                )
                .await
        }
    }
}

async fn project(
    state: &UiContextState,
    request_id: RequestId,
    request: Request<Body>,
) -> Result<Response<Body>, (StatusCode, &'static str)> {
    if request.uri().query().is_some() {
        return Err((StatusCode::BAD_REQUEST, "ui_context_query"));
    }
    let authority =
        request_authority(&request).map_err(|status| (status, "ui_context_authority"))?;
    let host = UiGenerationHost::parse(&authority, &state.namespace, state.public_port)
        .map_err(|_| (StatusCode::NOT_FOUND, "ui_not_found"))?;
    match state
        .host_resolver
        .resolve_active_generation_host(host)
        .await
        .map_err(|error| match error {
            UiHostLookupError::Unavailable => (StatusCode::SERVICE_UNAVAILABLE, "ui_unavailable"),
        })? {
        Some(ActiveUiGenerationHost { generation_id }) if generation_id == host.generation_id() => {
        }
        Some(_) | None => return Err((StatusCode::NOT_FOUND, "ui_not_found")),
    }
    require_same_origin(&request, host, &state.namespace, state.public_port, false)
        .map_err(|status| (status, "ui_context_origin"))?;
    let secret = parse_child_cookie(&request).map_err(|status| (status, "ui_context_cookie"))?;
    let target = state
        .projection
        .project_ui_target(request_id, secret, host.generation_id())
        .await
        .map_err(|error| match error {
            UiTargetContextError::Unauthorized => (StatusCode::FORBIDDEN, "ui_forbidden"),
            UiTargetContextError::Unavailable => {
                (StatusCode::SERVICE_UNAVAILABLE, "ui_unavailable")
            }
        })?;
    let context = verified_context(&target.context);
    let mut response = (
        StatusCode::OK,
        axum::Json(ContextResponse {
            repository_id: target.repository_id.as_uuid(),
        }),
    )
        .into_response();
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    response.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        axum::http::HeaderValue::from_static("nosniff"),
    );
    Ok(state
        .audit
        .allowed(
            request_id,
            release_service::UiRequestAuditSurface::Content,
            context,
            release_service::UiRequestAuditOutcome::Succeeded,
            release_service::UiRequestAuditReason::None,
            response,
        )
        .await)
}

async fn deny(
    state: &UiContextState,
    request_id: RequestId,
    status: StatusCode,
    code: &'static str,
) -> Response<Body> {
    state
        .audit
        .denial(
            request_id,
            release_service::UiRequestAuditSurface::Content,
            if status == StatusCode::SERVICE_UNAVAILABLE {
                release_service::UiRequestAuditReason::Unavailable
            } else if status == StatusCode::FORBIDDEN {
                release_service::UiRequestAuditReason::Unauthorized
            } else {
                release_service::UiRequestAuditReason::InvalidInput
            },
            error_response(status, code),
        )
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use axum::{body::to_bytes, extract::ConnectInfo, http::header};
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
    use forge_domain::OrganizationId;
    use identity_domain::UserId;
    use release_domain::{
        UiInstallationGenerationId, UiInstallationId,
        ui_browser::{UiBrowserRoute, UiBrowserSessionId},
    };
    use release_service::UiBrowserTargetContext;
    use std::{net::SocketAddr, sync::Arc};
    use tower::ServiceExt;
    use uuid::Uuid;

    struct Host {
        generation: UiInstallationGenerationId,
    }

    #[async_trait]
    impl UiGenerationHostResolver for Host {
        async fn resolve_active_generation_host(
            &self,
            host: UiGenerationHost,
        ) -> Result<Option<ActiveUiGenerationHost>, UiHostLookupError> {
            Ok(
                (host.generation_id() == self.generation).then_some(ActiveUiGenerationHost {
                    generation_id: self.generation,
                }),
            )
        }
    }

    struct Projection {
        target: UiBrowserTargetContext,
    }

    #[async_trait]
    impl UiBrowserTargetContextProjection for Projection {
        async fn project_ui_target(
            &self,
            _request_id: RequestId,
            _session_secret: release_domain::ui_browser::UiBrowserSessionSecret,
            _expected_generation_id: UiInstallationGenerationId,
        ) -> Result<UiBrowserTargetContext, UiTargetContextError> {
            Ok(self.target.clone())
        }
    }

    fn state(generation: UiInstallationGenerationId) -> Arc<UiContextState> {
        let context = release_service::UiBrowserSessionContext {
            session_id: UiBrowserSessionId::from_uuid(Uuid::new_v4()),
            parent_session_id: identity_domain::BrowserSessionId::from_uuid(Uuid::new_v4()),
            actor_id: UserId::new(),
            organization_id: OrganizationId::new(),
            installation_id: UiInstallationId::new(),
            generation_id: generation,
            route: UiBrowserRoute::parse("session-chat").expect("route"),
            expires_at: time::OffsetDateTime::now_utc() + time::Duration::hours(1),
        };
        Arc::new(UiContextState::new(
            Arc::new(Host { generation }),
            Arc::new(Projection {
                target: UiBrowserTargetContext {
                    context,
                    repository_id: forge_domain::RepositoryId::new(),
                },
            }),
            UiNamespace::parse("ui.example.test").expect("namespace"),
            UiPublicPort::https_default(),
            Arc::new(crate::ui_audit::NoopAuditSink),
        ))
    }

    fn request(generation: UiInstallationGenerationId, cookie: bool) -> Request<Body> {
        let host = UiGenerationHost::from_generation_id(generation);
        let namespace = UiNamespace::parse("ui.example.test").expect("namespace");
        let mut request = Request::builder()
            .uri(CONTEXT_PATH)
            .header(
                header::HOST,
                host.authority(&namespace, UiPublicPort::https_default()),
            )
            .body(Body::empty())
            .expect("context request");
        if cookie {
            let value = URL_SAFE_NO_PAD.encode([7_u8; 32]);
            request.headers_mut().insert(
                header::COOKIE,
                format!("{}={value}", release_service::UI_CHILD_COOKIE)
                    .parse()
                    .expect("cookie"),
            );
        }
        request
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 1234))));
        request
    }

    #[tokio::test]
    async fn route_returns_only_verified_repository_target() {
        let generation = UiInstallationGenerationId::from_uuid(Uuid::new_v4());
        let response = router(state(generation))
            .oneshot(request(generation, true))
            .await
            .expect("context response");
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        let body = to_bytes(response.into_body(), 1024).await.expect("body");
        let value: serde_json::Value = serde_json::from_slice(&body).expect("JSON");
        assert_eq!(value.as_object().expect("object").len(), 1);
        assert!(value["repository_id"].as_str().is_some());
    }

    #[tokio::test]
    async fn route_rejects_missing_child_cookie() {
        let generation = UiInstallationGenerationId::from_uuid(Uuid::new_v4());
        let response = router(state(generation))
            .oneshot(request(generation, false))
            .await
            .expect("context response");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
}
