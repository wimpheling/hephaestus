//! Focused tests for UI-origin bridge and middleware behavior.

use crate::ui_browser_content::{UiGatewayDispatchAuthority, UiGatewayDispatcher};
use async_trait::async_trait;
use axum::{Router, body::Body, routing::get};
use bytes::Bytes;
use forge_domain::OrganizationId;
use gateway_edge::{
    UiDispatchResult, UiGatewayAdmission, UiGatewayAdmissionError, UiGatewayAdmissionProvider,
    UiGatewayRequest, UiGatewayRequestKind,
};
use http::{HeaderMap, StatusCode};
use identity_domain::{RequestId, UserId};
use release_domain::{
    UiInstallationGenerationId, UiInstallationId, ui_browser::UiBrowserSessionId,
};
use release_service::{
    UiBrowserHttpPath, UiGatewayRequestKind as ReleaseGatewayRequestKind,
    UiGatewayRequestProjection, UiNamespace, UiPublicPort, UiRequestAuditDecision,
    UiRequestAuditOutcome, UiRequestAuditReason, UiRequestAuditSink, UiRequestAuditSurface,
};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tower::ServiceExt;
use uuid::Uuid;

use super::{
    bridge::{RealUiGatewayDispatcher, UiDispatchCore},
    router::{bounded_ui_router, bounded_ui_router_with_audit},
};

struct CapturingCore {
    request: Mutex<Option<UiGatewayRequest>>,
}

#[async_trait]
impl UiDispatchCore for CapturingCore {
    async fn dispatch_ui_request_detailed(
        &self,
        request: UiGatewayRequest,
        _provider: &dyn UiGatewayAdmissionProvider,
    ) -> UiDispatchResult {
        *self.request.lock().expect("capture lock") = Some(request);
        UiDispatchResult {
            response: gateway_edge::GatewayProviderResponse {
                response: gateway_edge::GatewayResponse {
                    status: StatusCode::OK,
                    headers: HeaderMap::new(),
                    body: Bytes::from_static(b"ok"),
                    mailbox_publication: None,
                },
                invocation_id: Uuid::new_v4(),
            },
            disposition: gateway_edge::UiDispatchDisposition::Admitted {
                outcome: gateway_edge::GatewayInvocationOutcome::Completed,
                completion_persisted: true,
            },
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
        .dispatch_ui_detailed(
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
        .dispatch_ui_detailed(
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
async fn audited_outer_bound_records_permit_denial_before_response() {
    let permits = Arc::new(tokio::sync::Semaphore::new(1));
    let _held = permits.clone().try_acquire_owned().expect("permit");
    let sink = Arc::new(crate::ui_audit::CapturingAuditSink::default());
    let router = bounded_ui_router_with_audit(
        Router::new().route("/healthz", get(|| async { "ok" })),
        permits,
        Duration::from_secs(1),
        Arc::clone(&sink) as Arc<dyn UiRequestAuditSink>,
    );
    let response = router
        .oneshot(
            axum::http::Request::get("/healthz")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    let events = sink.events.lock().expect("events");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].surface(), UiRequestAuditSurface::Content);
    assert_eq!(events[0].decision(), UiRequestAuditDecision::Denied);
    assert_eq!(events[0].reason(), UiRequestAuditReason::Unavailable);
    assert!(events[0].context().actor_id().is_none());
    drop(events);
}

#[tokio::test]
async fn audited_outer_deadline_records_unknown_after_operation_may_have_started() {
    let sink = Arc::new(crate::ui_audit::CapturingAuditSink::default());
    let router = bounded_ui_router_with_audit(
        Router::new().route(
            "/healthz",
            get(|| async {
                tokio::time::sleep(Duration::from_millis(20)).await;
                "late"
            }),
        ),
        Arc::new(tokio::sync::Semaphore::new(1)),
        Duration::from_millis(1),
        Arc::clone(&sink) as Arc<dyn UiRequestAuditSink>,
    );
    let response = router
        .oneshot(
            axum::http::Request::get("/healthz")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let events = sink.events.lock().expect("events");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].decision(), UiRequestAuditDecision::Undetermined);
    assert_eq!(events[0].outcome(), UiRequestAuditOutcome::Unknown);
    assert_eq!(events[0].reason(), UiRequestAuditReason::Unavailable);
    drop(events);
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
