use super::UI_BROWSER_SESSION_TTL_SECONDS;
use time::{Duration, OffsetDateTime};

/// Scoped failure categories for handoff exchange.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum UiBrowserHandoffFailure {
    /// The secret was invalid, expired, already consumed, or bound elsewhere.
    #[error("UI browser handoff is invalid or expired")]
    InvalidOrExpired,
    /// The parent or target generation is no longer eligible.
    #[error("UI browser handoff is unavailable")]
    Unavailable,
}

/// Scoped failure categories for validating a child browser session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum UiBrowserSessionFailure {
    /// The child secret, parent, organization, or generation binding is not
    /// valid for this request.
    #[error("UI browser session is unauthenticated")]
    Unauthenticated,
    /// The bound generation is no longer enabled for this installation.
    #[error("UI browser session is unavailable")]
    Unavailable,
    /// The requested lifetime cannot be represented safely.
    #[error("UI browser session lifetime is invalid")]
    InvalidLifetime,
}

/// Returns the child-session expiry, capped by both the fixed twelve-hour
/// child lifetime and the parent session expiry. There is deliberately no
/// sliding-renewal path.
///
/// # Errors
///
/// Returns [`UiBrowserSessionFailure::Unauthenticated`] when the parent is
/// already expired at the child issue instant, or
/// [`UiBrowserSessionFailure::InvalidLifetime`] if time arithmetic overflows.
pub fn child_session_expiry(
    issued_at: OffsetDateTime,
    parent_expires_at: OffsetDateTime,
) -> Result<OffsetDateTime, UiBrowserSessionFailure> {
    if parent_expires_at <= issued_at {
        return Err(UiBrowserSessionFailure::Unauthenticated);
    }
    let child_expires_at = issued_at
        .checked_add(Duration::seconds(UI_BROWSER_SESSION_TTL_SECONDS))
        .ok_or(UiBrowserSessionFailure::InvalidLifetime)?;
    Ok(child_expires_at.min(parent_expires_at))
}
