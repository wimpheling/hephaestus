use identity_domain::{BrowserSessionId, BrowserSessionSid, RequestId, UserId};
use release_service::{NewUiRequestAuditEvent, UiRequestAuditSink};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::time::timeout;
use uuid::Uuid;

const ISSUER: &str = "hephaestus-web-mediator";
const BOOTSTRAP_SUBJECT: &str = "hephaestus-web-mediator";
const BOOTSTRAP_ACTOR_KIND: &str = "verified_oidc_bootstrap";
const BOOTSTRAP_AUDIENCE: &str = "/hephaestus.identity.v1.IdentityService/ResolveIdentity";
const CREATE_BOOTSTRAP_AUDIENCE: &str =
    "/hephaestus.identity.v1.IdentityService/CreateBrowserSession";
const REVOKE_AUDIENCE: &str = "/hephaestus.identity.v1.IdentityService/RevokeBrowserSession";
const HANDOFF_AUDIENCE: &str = "/hephaestus.release.v1.ReleaseService/CreateUiBrowserHandoff";
const MAX_LIFETIME_SECONDS: i64 = 30;
const CLOCK_SKEW_SECONDS: i64 = 5;
// Worker-pool acquisition and the audit transaction can exceed 250 ms under
// shared PostgreSQL test and startup load. Keep the append bounded while
// leaving enough time for one normal transaction to complete.
const UI_AUDIT_APPEND_BUDGET: Duration = Duration::from_millis(500);

/// Request-local state shared by the authentication layer, Connect dispatch,
/// and the handoff handler. The generated ID is correlation-only: it never
/// becomes verified actor or target context.
#[derive(Clone)]
pub struct UiHandoffAuditMarker {
    request_id: RequestId,
    handler_reached: Arc<AtomicBool>,
    actor_id: Arc<Mutex<Option<UserId>>>,
}

impl UiHandoffAuditMarker {
    fn new() -> Self {
        Self {
            request_id: RequestId::new(),
            handler_reached: Arc::new(AtomicBool::new(false)),
            actor_id: Arc::new(Mutex::new(None)),
        }
    }

    pub(crate) const fn request_id(&self) -> RequestId {
        self.request_id
    }

    pub(crate) fn mark_handler_reached(&self) {
        self.handler_reached.store(true, Ordering::Release);
    }

    pub(crate) fn set_actor(&self, actor_id: UserId) {
        if let Ok(mut value) = self.actor_id.lock() {
            *value = Some(actor_id);
        }
    }

    fn actor_id(&self) -> Option<UserId> {
        self.actor_id.lock().ok().and_then(|value| *value)
    }

    fn handler_reached(&self) -> bool {
        self.handler_reached.load(Ordering::Acquire)
    }
}

/// Append one audit event without allowing an unavailable audit sink to alter
/// the already-determined RPC result. The timeout also prevents a stalled
/// worker pool from holding the RPC request open indefinitely.
pub async fn append_ui_request_audit_bounded(
    sink: &dyn UiRequestAuditSink,
    event: NewUiRequestAuditEvent,
) -> bool {
    let request_id = event.request_id();
    let surface = event.surface();
    let reason = event.reason();
    if timeout(UI_AUDIT_APPEND_BUDGET, sink.append(event)).await == Ok(Ok(())) {
        true
    } else {
        tracing::warn!(
            %request_id,
            %surface,
            reason = reason.as_str(),
            "UI request audit append unavailable or timed out"
        );
        false
    }
}

/// Authenticated mediator subject safe to convert into application identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MediatorPrincipal {
    /// Internal user selected by the trusted mediator assertion.
    pub user_id: UserId,
    /// Unique assertion identifier retained only for audit correlation.
    pub assertion_id: Uuid,
    /// Browser SID carried by the signed mediator assertion.
    pub sid: BrowserSessionSid,
}

/// Identity fields a bootstrap assertion must bind to the request body.
pub struct BootstrapIdentity<'a> {
    /// Verified external OIDC issuer.
    pub issuer: &'a str,
    /// Verified external OIDC subject.
    pub subject: &'a str,
    /// Display name received from the verified identity.
    pub display_name: &'a str,
    /// Email received from the verified identity.
    pub email: &'a str,
    /// Whether the upstream issuer verified the email.
    pub email_verified: bool,
}

/// Typed mediator session claims installed by the edge.
///
/// Normal RPCs carry claims that have passed the durable active-session check.
/// Revoke carries signed claims only so an expired session can self-revoke; the
/// revoke handler must not treat that path as proof of current activity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerifiedMediatorSession {
    /// Internal user selected by the signed mediator assertion.
    pub user_id: UserId,
    /// Unique assertion identifier retained only for audit correlation.
    pub assertion_id: Uuid,
    /// Browser SID carried by the signed assertion; active routes verify it
    /// against `PostgreSQL` before admitting the request.
    pub sid: BrowserSessionSid,
    /// Internal durable row identity returned by the active-session check.
    ///
    /// Signed-only routes such as revoke intentionally leave this absent;
    /// only active middleware authentication can populate it.
    pub parent_session_id: Option<BrowserSessionId>,
}

mod authenticator;
mod claims;
mod middleware;
mod state;

pub use authenticator::MediatorAuthenticator;
pub use claims::MediatorAssertionError;
#[cfg(test)]
use claims::requires_mediator_auth;
pub use middleware::mediator_identity_middleware;
pub use state::MediatorAuthenticationState;

#[cfg(test)]
mod tests;
