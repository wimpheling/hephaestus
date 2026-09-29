//! Bounded UI-origin request middleware.

use axum::{Router, middleware, middleware::Next};
use http::StatusCode;
use identity_domain::RequestId;
use release_service::{
    UiRequestAuditContext, UiRequestAuditReason, UiRequestAuditSink, UiRequestAuditSurface,
};
use std::{sync::Arc, time::Duration};

use crate::ui_audit::{UiAuditRecorder, UiRequestCorrelation};

#[cfg(test)]
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

/// Audited outer bound used by the production composition root. It allocates
/// correlation before permit/deadline denial; inner handlers own verified
/// success and protected-operation outcomes.
pub fn bounded_ui_router_with_audit<S>(
    router: Router<S>,
    permits: Arc<tokio::sync::Semaphore>,
    deadline: Duration,
    audit: Arc<dyn UiRequestAuditSink>,
) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    let audit = Arc::new(UiAuditRecorder::new(audit));
    router.layer(middleware::from_fn(
        move |request: http::Request<axum::body::Body>, next: Next| {
            let permits = Arc::clone(&permits);
            let audit = Arc::clone(&audit);
            async move {
                let request_id = RequestId::new();
                let mut request = request;
                request
                    .extensions_mut()
                    .insert(UiRequestCorrelation(request_id));
                let surface = if request.uri().path()
                    == release_service::ui_browser_host::UI_BOOTSTRAP_PATH
                {
                    UiRequestAuditSurface::Bootstrap
                } else {
                    UiRequestAuditSurface::Content
                };
                let denied = |status: StatusCode, reason: UiRequestAuditReason| {
                    let audit = Arc::clone(&audit);
                    async move {
                        audit
                            .denial(
                                request_id,
                                surface,
                                reason,
                                axum::response::Response::builder()
                                    .status(status)
                                    .header(http::header::CACHE_CONTROL, "no-store")
                                    .body(axum::body::Body::empty())
                                    .expect("bounded UI response"),
                            )
                            .await
                    }
                };
                let Ok(_permit) = permits.try_acquire_owned() else {
                    return denied(
                        StatusCode::TOO_MANY_REQUESTS,
                        UiRequestAuditReason::Unavailable,
                    )
                    .await;
                };
                match tokio::time::timeout(deadline, next.run(request)).await {
                    Ok(response) => response,
                    Err(_) => {
                        audit
                            .undetermined(
                                request_id,
                                surface,
                                UiRequestAuditContext::anonymous(),
                                axum::response::Response::builder()
                                    .status(StatusCode::SERVICE_UNAVAILABLE)
                                    .header(http::header::CACHE_CONTROL, "no-store")
                                    .body(axum::body::Body::empty())
                                    .expect("bounded UI timeout response"),
                            )
                            .await
                    }
                }
            }
        },
    ))
}
