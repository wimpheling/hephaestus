//! Provider-neutral, redacted audit evidence for release-owned UI requests.
//!
//! This boundary contains only closed vocabularies and opaque identities. It
//! has no fields for parent session IDs, bearer digests, credentials, paths,
//! queries, headers, bodies, or provider responses.

use async_trait::async_trait;
use gateway_domain::{GatewayId, GatewayRevisionId};
use identity_domain::{OrganizationId, RequestId, UserId};
use release_domain::{
    UiInstallationGenerationId, UiInstallationId, ui_browser::UiBrowserSessionId,
};
use std::fmt;
use thiserror::Error;
use time::OffsetDateTime;
use uuid::Uuid;

/// The bounded UI request phase represented by one audit row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiRequestAuditSurface {
    /// Trusted Phoenix request that creates a one-time handoff.
    HandoffIssue,
    /// Trusted bootstrap exchange that consumes a handoff.
    HandoffExchange,
    /// Host bootstrap resolution or document bootstrap phase.
    Bootstrap,
    /// Content request denied before a static/managed/API declaration was
    /// safely classified, such as an unknown host or invalid child cookie.
    Content,
    /// Static artifact content serving.
    Static,
    /// Managed-service forwarding.
    Managed,
    /// Authenticated gateway API forwarding.
    Api,
    /// Authorized embed or launch intent. This is not proof that a browser
    /// rendered an iframe; actual content requests use their own surface.
    Embed,
}

impl UiRequestAuditSurface {
    /// Returns the stable database representation.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::HandoffIssue => "handoff_issue",
            Self::HandoffExchange => "handoff_exchange",
            Self::Bootstrap => "bootstrap",
            Self::Content => "content",
            Self::Static => "static",
            Self::Managed => "managed",
            Self::Api => "api",
            Self::Embed => "embed",
        }
    }
}

/// Decision made by the current authority boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiRequestAuditDecision {
    /// The request passed the relevant current authority checks.
    Allowed,
    /// The request was rejected before the protected operation.
    Denied,
    /// The bounded operation may have started, but its final disposition is
    /// unknowable after a deadline or transport cancellation.
    Undetermined,
}

impl UiRequestAuditDecision {
    /// Returns the stable database representation.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Allowed => "allowed",
            Self::Denied => "denied",
            Self::Undetermined => "undetermined",
        }
    }
}

/// Result of the bounded phase after its decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiRequestAuditOutcome {
    /// The allowed phase committed or served successfully.
    Succeeded,
    /// The allowed phase failed after authorization.
    Failed,
    /// The denied phase did not perform the protected operation.
    NotAttempted,
    /// The operation may have executed, but completion could not be observed.
    Unknown,
}

impl UiRequestAuditOutcome {
    /// Returns the stable database representation.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::NotAttempted => "not_attempted",
            Self::Unknown => "unknown",
        }
    }
}

/// Closed, non-sensitive reason vocabulary for one decision or outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiRequestAuditReason {
    /// No additional reason was needed.
    None,
    /// Authentication context was absent or malformed.
    Unauthenticated,
    /// Current authority denied the request.
    Unauthorized,
    /// The typed request shape was invalid.
    InvalidInput,
    /// The requested published route was not declared.
    InvalidRoute,
    /// A handoff or session was expired.
    Expired,
    /// A parent, installation, or generation was revoked or disabled.
    Revoked,
    /// The expected immutable generation did not match.
    GenerationMismatch,
    /// The safe referenced object was not found.
    NotFound,
    /// The persistence boundary was unavailable.
    Unavailable,
    /// The authorized upstream operation failed.
    UpstreamFailure,
}

impl UiRequestAuditReason {
    /// Returns the stable database representation.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Unauthenticated => "unauthenticated",
            Self::Unauthorized => "unauthorized",
            Self::InvalidInput => "invalid_input",
            Self::InvalidRoute => "invalid_route",
            Self::Expired => "expired",
            Self::Revoked => "revoked",
            Self::GenerationMismatch => "generation_mismatch",
            Self::NotFound => "not_found",
            Self::Unavailable => "unavailable",
            Self::UpstreamFailure => "upstream_failure",
        }
    }
}

/// Safe verified references attached to an audit event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UiRequestAuditContext {
    actor_id: Option<UserId>,
    organization_id: Option<OrganizationId>,
    installation_id: Option<UiInstallationId>,
    generation_id: Option<UiInstallationGenerationId>,
    child_session_id: Option<UiBrowserSessionId>,
    gateway: Option<(GatewayId, GatewayRevisionId)>,
}

impl UiRequestAuditContext {
    /// Creates anonymous context for malformed or unverified requests.
    #[must_use]
    pub const fn anonymous() -> Self {
        Self {
            actor_id: None,
            organization_id: None,
            installation_id: None,
            generation_id: None,
            child_session_id: None,
            gateway: None,
        }
    }

    /// Creates context with only an actor established by trusted middleware.
    #[must_use]
    pub const fn actor(actor_id: UserId) -> Self {
        Self {
            actor_id: Some(actor_id),
            ..Self::anonymous()
        }
    }

    /// Creates context after the installation and generation were verified.
    #[must_use]
    pub const fn verified(
        actor_id: UserId,
        organization_id: OrganizationId,
        installation_id: UiInstallationId,
        generation_id: UiInstallationGenerationId,
        child_session_id: Option<UiBrowserSessionId>,
        gateway: Option<(GatewayId, GatewayRevisionId)>,
    ) -> Self {
        Self {
            actor_id: Some(actor_id),
            organization_id: Some(organization_id),
            installation_id: Some(installation_id),
            generation_id: Some(generation_id),
            child_session_id,
            gateway,
        }
    }

    /// Returns the verified actor, if one exists.
    #[must_use]
    pub const fn actor_id(self) -> Option<UserId> {
        self.actor_id
    }

    /// Returns the verified organization, if one exists.
    #[must_use]
    pub const fn organization_id(self) -> Option<OrganizationId> {
        self.organization_id
    }

    /// Returns the verified installation, if one exists.
    #[must_use]
    pub const fn installation_id(self) -> Option<UiInstallationId> {
        self.installation_id
    }

    /// Returns the verified generation, if one exists.
    #[must_use]
    pub const fn generation_id(self) -> Option<UiInstallationGenerationId> {
        self.generation_id
    }

    /// Returns the verified child session, if one exists.
    #[must_use]
    pub const fn child_session_id(self) -> Option<UiBrowserSessionId> {
        self.child_session_id
    }

    /// Returns the verified gateway, if one exists.
    #[must_use]
    pub const fn gateway_id(self) -> Option<GatewayId> {
        match self.gateway {
            Some((gateway_id, _)) => Some(gateway_id),
            None => None,
        }
    }

    /// Returns the immutable gateway revision bound to the generation.
    #[must_use]
    pub const fn gateway_revision_id(self) -> Option<GatewayRevisionId> {
        match self.gateway {
            Some((_, gateway_revision_id)) => Some(gateway_revision_id),
            None => None,
        }
    }
}

/// One immutable redacted UI request audit event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NewUiRequestAuditEvent {
    id: Uuid,
    request_id: RequestId,
    surface: UiRequestAuditSurface,
    decision: UiRequestAuditDecision,
    outcome: UiRequestAuditOutcome,
    reason: UiRequestAuditReason,
    context: UiRequestAuditContext,
    occurred_at: OffsetDateTime,
}

impl NewUiRequestAuditEvent {
    /// Creates an event with an explicit occurrence time for transactional
    /// adapters and deterministic tests.
    #[must_use]
    pub fn new_at(
        request_id: RequestId,
        surface: UiRequestAuditSurface,
        decision: UiRequestAuditDecision,
        outcome: UiRequestAuditOutcome,
        reason: UiRequestAuditReason,
        context: UiRequestAuditContext,
        occurred_at: OffsetDateTime,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            request_id,
            surface,
            decision,
            outcome,
            reason,
            context,
            occurred_at,
        }
    }

    /// Creates an event timestamped at the current UTC instant.
    #[must_use]
    pub fn now(
        request_id: RequestId,
        surface: UiRequestAuditSurface,
        decision: UiRequestAuditDecision,
        outcome: UiRequestAuditOutcome,
        reason: UiRequestAuditReason,
        context: UiRequestAuditContext,
    ) -> Self {
        Self::new_at(
            request_id,
            surface,
            decision,
            outcome,
            reason,
            context,
            OffsetDateTime::now_utc(),
        )
    }

    /// Returns the immutable event ID.
    #[must_use]
    pub const fn id(self) -> Uuid {
        self.id
    }

    /// Returns the request correlation ID. It is correlation only and is not
    /// treated as proof of an actor or target.
    #[must_use]
    pub const fn request_id(self) -> RequestId {
        self.request_id
    }

    /// Returns the event surface.
    #[must_use]
    pub const fn surface(self) -> UiRequestAuditSurface {
        self.surface
    }

    /// Returns the authorization decision.
    #[must_use]
    pub const fn decision(self) -> UiRequestAuditDecision {
        self.decision
    }

    /// Returns the phase outcome.
    #[must_use]
    pub const fn outcome(self) -> UiRequestAuditOutcome {
        self.outcome
    }

    /// Returns the closed reason.
    #[must_use]
    pub const fn reason(self) -> UiRequestAuditReason {
        self.reason
    }

    /// Returns verified safe context only.
    #[must_use]
    pub const fn context(self) -> UiRequestAuditContext {
        self.context
    }

    /// Returns the event occurrence time.
    #[must_use]
    pub const fn occurred_at(self) -> OffsetDateTime {
        self.occurred_at
    }
}

/// Provider-neutral sink for one immutable UI request audit event.
#[async_trait]
pub trait UiRequestAuditSink: Send + Sync {
    /// Appends one event in its own transaction.
    async fn append(&self, event: NewUiRequestAuditEvent) -> Result<(), UiRequestAuditError>;
}

/// Errors from the audit persistence boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum UiRequestAuditError {
    /// The audit store could not commit the event.
    #[error("UI request audit persistence is unavailable")]
    Unavailable,
}

impl fmt::Display for UiRequestAuditSurface {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anonymous_denial_has_no_verified_subject_or_target() {
        let context = UiRequestAuditContext::anonymous();
        let event = NewUiRequestAuditEvent::new_at(
            RequestId::new(),
            UiRequestAuditSurface::Bootstrap,
            UiRequestAuditDecision::Denied,
            UiRequestAuditOutcome::NotAttempted,
            UiRequestAuditReason::Unauthenticated,
            context,
            OffsetDateTime::UNIX_EPOCH,
        );
        assert_eq!(event.context().actor_id(), None);
        assert_eq!(event.context().organization_id(), None);
        assert_eq!(event.context().installation_id(), None);
        assert_eq!(event.context().generation_id(), None);
        assert_eq!(event.context().child_session_id(), None);
        assert_eq!(event.context().gateway_id(), None);
        assert_eq!(event.context().gateway_revision_id(), None);
        assert!(!format!("{event:?}").contains("secret"));
    }

    #[test]
    fn closed_vocabularies_have_stable_values() {
        assert_eq!(UiRequestAuditSurface::Embed.as_str(), "embed");
        assert_eq!(UiRequestAuditDecision::Allowed.as_str(), "allowed");
        assert_eq!(
            UiRequestAuditOutcome::NotAttempted.as_str(),
            "not_attempted"
        );
        assert_eq!(
            UiRequestAuditReason::GenerationMismatch.as_str(),
            "generation_mismatch"
        );
        assert_eq!(
            UiRequestAuditDecision::Undetermined.as_str(),
            "undetermined"
        );
        assert_eq!(UiRequestAuditOutcome::Unknown.as_str(), "unknown");
        assert_eq!(UiRequestAuditReason::Unavailable.as_str(), "unavailable");
    }
}
