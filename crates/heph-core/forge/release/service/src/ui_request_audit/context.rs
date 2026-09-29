use gateway_domain::{GatewayId, GatewayRevisionId};
use identity_domain::{OrganizationId, RequestId, UserId};
use release_domain::{
    UiInstallationGenerationId, UiInstallationId, ui_browser::UiBrowserSessionId,
};
use time::OffsetDateTime;
use uuid::Uuid;

use super::{
    UiRequestAuditDecision, UiRequestAuditOutcome, UiRequestAuditReason, UiRequestAuditSurface,
};

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
