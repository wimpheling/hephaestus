use super::{
    MediatorAssertionError, MediatorAuthenticator, UiHandoffAuditMarker, UiRequestAuditSink,
    VerifiedMediatorSession, append_ui_request_audit_bounded,
};
use axum::http::HeaderMap;
use identity_application::{BrowserSessionAuthenticationError, BrowserSessionStore};
use release_service::{
    NewUiRequestAuditEvent, UiRequestAuditContext, UiRequestAuditDecision, UiRequestAuditOutcome,
    UiRequestAuditReason, UiRequestAuditSurface,
};
use std::sync::Arc;

/// Error classes returned by the durable browser-session check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SessionAuthenticationError {
    Unauthenticated,
    Unavailable,
}

/// Cloneable middleware state for signed mediator and browser-session checks.
#[derive(Clone)]
pub struct MediatorAuthenticationState {
    authenticator: MediatorAuthenticator,
    browser_sessions: Arc<dyn BrowserSessionStore>,
    ui_request_audit: Option<Arc<dyn UiRequestAuditSink>>,
}

impl MediatorAuthenticationState {
    /// Creates middleware state with the application-role session verifier.
    #[must_use]
    pub fn new(
        authenticator: MediatorAuthenticator,
        browser_sessions: Arc<dyn BrowserSessionStore>,
    ) -> Self {
        Self {
            authenticator,
            browser_sessions,
            ui_request_audit: None,
        }
    }

    /// Adds the worker-owned sink used for denied handoff authentication
    /// attempts. Other middleware paths remain audit-neutral.
    #[must_use]
    pub fn with_ui_request_audit_sink(mut self, sink: Arc<dyn UiRequestAuditSink>) -> Self {
        self.ui_request_audit = Some(sink);
        self
    }

    pub(super) async fn audit_handoff_denial(
        &self,
        marker: &UiHandoffAuditMarker,
        reason: UiRequestAuditReason,
    ) {
        let Some(sink) = &self.ui_request_audit else {
            return;
        };
        if marker.handler_reached() {
            return;
        }
        let event = NewUiRequestAuditEvent::now(
            marker.request_id(),
            UiRequestAuditSurface::HandoffIssue,
            UiRequestAuditDecision::Denied,
            UiRequestAuditOutcome::NotAttempted,
            reason,
            marker.actor_id().map_or_else(
                UiRequestAuditContext::anonymous,
                UiRequestAuditContext::actor,
            ),
        );
        let _ = append_ui_request_audit_bounded(sink.as_ref(), event).await;
    }

    pub(super) fn authenticate_signed(
        &self,
        headers: &HeaderMap,
        expected_audience: &str,
    ) -> Result<VerifiedMediatorSession, MediatorAssertionError> {
        let principal = self
            .authenticator
            .authenticate(headers, expected_audience)?;
        Ok(VerifiedMediatorSession {
            user_id: principal.user_id,
            assertion_id: principal.assertion_id,
            sid: principal.sid,
            parent_session_id: None,
        })
    }

    pub(super) async fn authenticate_active(
        &self,
        headers: &HeaderMap,
        expected_audience: &str,
    ) -> Result<VerifiedMediatorSession, SessionAuthenticationError> {
        let session = self
            .authenticate_signed(headers, expected_audience)
            .map_err(|_| SessionAuthenticationError::Unauthenticated)?;
        let metadata = self
            .browser_sessions
            .authenticate_browser_session(session.user_id, session.sid)
            .await
            .map_err(|error| match error {
                BrowserSessionAuthenticationError::Unauthenticated => {
                    SessionAuthenticationError::Unauthenticated
                }
                BrowserSessionAuthenticationError::Unavailable => {
                    SessionAuthenticationError::Unavailable
                }
            })?;
        if metadata.user_id() != session.user_id {
            return Err(SessionAuthenticationError::Unauthenticated);
        }
        Ok(VerifiedMediatorSession {
            parent_session_id: Some(metadata.id()),
            ..session
        })
    }
}
