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
