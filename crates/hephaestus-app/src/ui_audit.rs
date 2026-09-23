//! Audited UI-origin handler boundary for the external HTTP draft.
//!
//! Only the closed release-service audit vocabulary crosses this helper.  It
//! deliberately has no request path, query, header, body, handoff secret, or
//! parent-session field.

#[cfg(test)]
use async_trait::async_trait;
use axum::{http::Extensions, http::StatusCode, response::Response};
use identity_domain::RequestId;
use release_service::{
    NewUiRequestAuditEvent, UiBrowserSessionContext, UiRequestAuditContext, UiRequestAuditDecision,
    UiRequestAuditOutcome, UiRequestAuditReason, UiRequestAuditSink, UiRequestAuditSurface,
};
use std::{sync::Arc, time::Duration};

/// Small independent budget for one audit append. It is intentionally shorter
/// than the total UI operation deadline and is applied on every path,
/// including denials outside the inner handler.
pub const DEFAULT_AUDIT_APPEND_TIMEOUT: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AuditAppendFailure {
    Timeout,
    SinkUnavailable,
}

impl AuditAppendFailure {
    const fn error_class(self) -> &'static str {
        match self {
            Self::Timeout => "timeout",
            Self::SinkUnavailable => "sink-unavailable",
        }
    }
}

/// Correlation allocated once by the outer UI middleware and read by inner
/// handlers. It carries no authority and is never serialized.
#[derive(Clone, Copy)]
pub struct UiRequestCorrelation(pub RequestId);

/// Reads the outer correlation. The fallback exists only for direct unit
/// router calls; the production composition always inserts the extension
/// before either handler runs.
pub fn correlation_id(extensions: &Extensions) -> RequestId {
    extensions
        .get::<UiRequestCorrelation>()
        .map_or_else(RequestId::new, |correlation| correlation.0)
}

/// Small handler-facing audit facade.  The app composition root must install
/// `PgUiRequestAuditRepository` over the existing worker pool.
#[derive(Clone)]
pub struct UiAuditRecorder {
    sink: Arc<dyn UiRequestAuditSink>,
    append_timeout: Duration,
}

impl UiAuditRecorder {
    pub fn new(sink: Arc<dyn UiRequestAuditSink>) -> Self {
        Self::with_append_timeout(sink, DEFAULT_AUDIT_APPEND_TIMEOUT)
    }

    pub fn with_append_timeout(
        sink: Arc<dyn UiRequestAuditSink>,
        append_timeout: Duration,
    ) -> Self {
        Self {
            sink,
            append_timeout,
        }
    }

    async fn append(&self, event: NewUiRequestAuditEvent) -> Result<(), AuditAppendFailure> {
        match tokio::time::timeout(self.append_timeout, self.sink.append(event)).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(_)) => Err(AuditAppendFailure::SinkUnavailable),
            Err(_) => Err(AuditAppendFailure::Timeout),
        }
    }

    /// Appends a denial while retaining the original safe denial response if
    /// audit persistence is unavailable.  The warning carries only the
    /// correlation ID and a closed surface label.
    pub async fn denial(
        &self,
        request_id: RequestId,
        surface: UiRequestAuditSurface,
        reason: UiRequestAuditReason,
        response: Response,
    ) -> Response {
        self.denial_with_context(
            request_id,
            surface,
            UiRequestAuditContext::anonymous(),
            reason,
            response,
        )
        .await
    }

    /// Appends a denied event after safe verified context exists while
    /// retaining the original denial response.
    pub async fn denial_with_context(
        &self,
        request_id: RequestId,
        surface: UiRequestAuditSurface,
        context: UiRequestAuditContext,
        reason: UiRequestAuditReason,
        response: Response,
    ) -> Response {
        let event = NewUiRequestAuditEvent::now(
            request_id,
            surface,
            UiRequestAuditDecision::Denied,
            UiRequestAuditOutcome::NotAttempted,
            reason,
            context,
        );
        if let Err(failure) = self.append(event).await {
            tracing::warn!(
                request_id = %request_id,
                surface = %surface,
                stage = "ui-audit-append",
                error_class = failure.error_class(),
                "UI denial audit append failed"
            );
        }
        response
    }

    /// Records a timeout whose protected operation may have started. It never
    /// uses the denied/not-attempted pair.
    pub async fn undetermined(
        &self,
        request_id: RequestId,
        surface: UiRequestAuditSurface,
        context: UiRequestAuditContext,
        response: Response,
    ) -> Response {
        let event = NewUiRequestAuditEvent::now(
            request_id,
            surface,
            UiRequestAuditDecision::Undetermined,
            UiRequestAuditOutcome::Unknown,
            UiRequestAuditReason::Unavailable,
            context,
        );
        if let Err(failure) = self.append(event).await {
            tracing::warn!(
                request_id = %request_id,
                surface = %surface,
                stage = "ui-audit-append",
                error_class = failure.error_class(),
                "UI timeout audit append failed"
            );
        }
        response
    }

    /// Appends an allowed phase before its response is returned to Axum.  A
    /// failed append converts the response to a generic unavailable result so
    /// successful bytes are never sent without durable evidence.
    pub async fn allowed(
        &self,
        request_id: RequestId,
        surface: UiRequestAuditSurface,
        context: UiRequestAuditContext,
        outcome: UiRequestAuditOutcome,
        reason: UiRequestAuditReason,
        response: Response,
    ) -> Response {
        let event = NewUiRequestAuditEvent::now(
            request_id,
            surface,
            UiRequestAuditDecision::Allowed,
            outcome,
            reason,
            context,
        );
        if let Err(failure) = self.append(event).await {
            tracing::warn!(
                request_id = %request_id,
                surface = %surface,
                stage = "ui-audit-append",
                error_class = failure.error_class(),
                "UI allowed audit append failed"
            );
            return audit_unavailable();
        }
        response
    }

    /// Records an authorized operation failure.  The operation may already
    /// have reached a guest gateway; the event therefore says `failed` and
    /// preserves the redacted failure response rather than claiming success.
    pub async fn failed(
        &self,
        request_id: RequestId,
        surface: UiRequestAuditSurface,
        context: UiRequestAuditContext,
        reason: UiRequestAuditReason,
        response: Response,
    ) -> Response {
        self.allowed(
            request_id,
            surface,
            context,
            UiRequestAuditOutcome::Failed,
            reason,
            response,
        )
        .await
    }
}

/// Converts a verified safe child context into the audit context.  The parent
/// session identity is intentionally not copied because it is not a safe
/// serving reference in the public audit type.
pub const fn verified_context(context: &UiBrowserSessionContext) -> UiRequestAuditContext {
    UiRequestAuditContext::verified(
        context.actor_id,
        context.organization_id,
        context.installation_id,
        context.generation_id,
        Some(context.session_id),
        None,
    )
}

pub const fn reason_for_content_error(
    error: crate::ui_browser_content::UiContentError,
) -> release_service::UiRequestAuditReason {
    use crate::ui_browser_content::UiContentError;
    match error {
        UiContentError::InvalidRequest | UiContentError::BodyTooLarge => {
            release_service::UiRequestAuditReason::InvalidInput
        }
        UiContentError::NotFound => release_service::UiRequestAuditReason::NotFound,
        UiContentError::Unauthenticated => release_service::UiRequestAuditReason::Unauthenticated,
        UiContentError::RangeNotSatisfiable => release_service::UiRequestAuditReason::InvalidInput,
        UiContentError::GuestSetCookie | UiContentError::GuestRedirect => {
            release_service::UiRequestAuditReason::UpstreamFailure
        }
        UiContentError::Unavailable => release_service::UiRequestAuditReason::Unavailable,
    }
}

pub const fn reason_for_bootstrap_error(
    error: release_service::UiBrowserHandoffError,
) -> UiRequestAuditReason {
    use release_service::UiBrowserHandoffError;
    match error {
        UiBrowserHandoffError::PermissionDenied => UiRequestAuditReason::Unauthorized,
        UiBrowserHandoffError::InvalidRoute => UiRequestAuditReason::InvalidRoute,
        UiBrowserHandoffError::InvalidOrExpired => UiRequestAuditReason::Expired,
        UiBrowserHandoffError::Unavailable => UiRequestAuditReason::Unavailable,
    }
}

fn audit_unavailable() -> Response {
    let mut response = Response::new(axum::body::Body::from(r#"{"error":"ui_unavailable"}"#));
    *response.status_mut() = StatusCode::SERVICE_UNAVAILABLE;
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    response.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("application/json"),
    );
    response
}

/// Test/composition placeholder only; production wiring must use the worker
/// backed `PostgreSQL` repository.
#[cfg(test)]
pub struct NoopAuditSink;

#[cfg(test)]
#[async_trait]
impl UiRequestAuditSink for NoopAuditSink {
    async fn append(
        &self,
        _event: NewUiRequestAuditEvent,
    ) -> Result<(), release_service::UiRequestAuditError> {
        Ok(())
    }
}

/// Capturing sink used only by the external router tests.  It is intentionally
/// kept in this draft rather than exported by the production app.
#[cfg(test)]
#[derive(Default)]
pub struct CapturingAuditSink {
    pub events: std::sync::Mutex<Vec<NewUiRequestAuditEvent>>,
    pub fail: std::sync::atomic::AtomicBool,
}

#[cfg(test)]
#[async_trait]
impl UiRequestAuditSink for CapturingAuditSink {
    async fn append(
        &self,
        event: NewUiRequestAuditEvent,
    ) -> Result<(), release_service::UiRequestAuditError> {
        if self.fail.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(release_service::UiRequestAuditError::Unavailable);
        }
        self.events.lock().expect("capturing sink lock").push(event);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use release_service::UiRequestAuditError;

    struct NeverAuditSink;

    #[async_trait]
    impl UiRequestAuditSink for NeverAuditSink {
        async fn append(&self, _event: NewUiRequestAuditEvent) -> Result<(), UiRequestAuditError> {
            std::future::pending().await
        }
    }

    struct UnavailableAuditSink;

    #[async_trait]
    impl UiRequestAuditSink for UnavailableAuditSink {
        async fn append(&self, _event: NewUiRequestAuditEvent) -> Result<(), UiRequestAuditError> {
            Err(UiRequestAuditError::Unavailable)
        }
    }

    fn audit_event() -> NewUiRequestAuditEvent {
        NewUiRequestAuditEvent::now(
            RequestId::new(),
            UiRequestAuditSurface::Content,
            UiRequestAuditDecision::Allowed,
            UiRequestAuditOutcome::Succeeded,
            UiRequestAuditReason::None,
            UiRequestAuditContext::anonymous(),
        )
    }

    #[tokio::test]
    async fn audit_append_classifies_timeout_without_provider_details() {
        let recorder = UiAuditRecorder::with_append_timeout(
            std::sync::Arc::new(NeverAuditSink),
            Duration::from_millis(1),
        );

        let failure = recorder
            .append(audit_event())
            .await
            .expect_err("hanging sink must time out");

        assert_eq!(failure, AuditAppendFailure::Timeout);
        assert_eq!(failure.error_class(), "timeout");
    }

    #[tokio::test]
    async fn audit_append_classifies_closed_sink_failure() {
        let recorder = UiAuditRecorder::new(std::sync::Arc::new(UnavailableAuditSink));

        let failure = recorder
            .append(audit_event())
            .await
            .expect_err("unavailable sink must fail closed");

        assert_eq!(failure, AuditAppendFailure::SinkUnavailable);
        assert_eq!(failure.error_class(), "sink-unavailable");
    }

    #[tokio::test]
    async fn allowed_audit_failure_preserves_fail_closed_response() {
        let recorder = UiAuditRecorder::new(std::sync::Arc::new(UnavailableAuditSink));
        let response = recorder
            .allowed(
                RequestId::new(),
                UiRequestAuditSurface::Content,
                UiRequestAuditContext::anonymous(),
                UiRequestAuditOutcome::Succeeded,
                UiRequestAuditReason::None,
                Response::new(Body::empty()),
            )
            .await;

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn audit_append_timeout_retains_denial_without_hanging() {
        let recorder = UiAuditRecorder::with_append_timeout(
            std::sync::Arc::new(NeverAuditSink),
            Duration::from_millis(1),
        );
        let response = tokio::time::timeout(
            Duration::from_millis(100),
            recorder.denial(
                RequestId::new(),
                UiRequestAuditSurface::Content,
                UiRequestAuditReason::Unavailable,
                axum::response::Response::new(Body::empty()),
            ),
        )
        .await
        .expect("bounded denial audit");
        assert_eq!(response.status(), http::StatusCode::OK);
    }

    #[test]
    fn inner_correlation_reader_reuses_outer_request_id() {
        let request_id = RequestId::new();
        let mut extensions = Extensions::new();
        extensions.insert(UiRequestCorrelation(request_id));
        assert_eq!(correlation_id(&extensions), request_id);
    }
}
