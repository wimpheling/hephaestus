use super::super::UserId;
use super::BrowserSessionId;
use time::{Duration, OffsetDateTime};

/// Safe metadata returned after a session row is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BrowserSessionMetadata {
    id: BrowserSessionId,
    user_id: UserId,
    issued_at: OffsetDateTime,
    expires_at: OffsetDateTime,
    revoked_at: Option<OffsetDateTime>,
}

impl BrowserSessionMetadata {
    /// Constructs metadata while enforcing the domain lifetime invariants.
    #[must_use]
    pub fn new(
        id: BrowserSessionId,
        user_id: UserId,
        issued_at: OffsetDateTime,
        expires_at: OffsetDateTime,
        revoked_at: Option<OffsetDateTime>,
    ) -> Option<Self> {
        let lifetime = expires_at - issued_at;
        if lifetime <= Duration::ZERO
            || lifetime > Duration::seconds(super::MAX_BROWSER_SESSION_TTL_SECONDS)
            || revoked_at.is_some_and(|at| at < issued_at)
        {
            return None;
        }
        Some(Self {
            id,
            user_id,
            issued_at,
            expires_at,
            revoked_at,
        })
    }

    /// Returns the internal row identity.
    #[must_use]
    pub const fn id(self) -> BrowserSessionId {
        self.id
    }

    /// Returns the owning user.
    #[must_use]
    pub const fn user_id(self) -> UserId {
        self.user_id
    }

    /// Returns the issue instant.
    #[must_use]
    pub const fn issued_at(self) -> OffsetDateTime {
        self.issued_at
    }

    /// Returns the expiry instant.
    #[must_use]
    pub const fn expires_at(self) -> OffsetDateTime {
        self.expires_at
    }

    /// Returns the irreversible revocation instant, if present.
    #[must_use]
    pub const fn revoked_at(self) -> Option<OffsetDateTime> {
        self.revoked_at
    }

    /// Checks the durable session state at one server-provided instant.
    #[must_use]
    pub fn is_active_at(self, now: OffsetDateTime) -> bool {
        self.revoked_at.is_none() && now >= self.issued_at && now < self.expires_at
    }
}

/// Closed set of non-sensitive browser-session revocation reasons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserSessionRevocationReason {
    /// The browser explicitly logged out.
    Logout,
    /// An operator or account policy revoked the session.
    Administrative,
    /// The session was revoked as a security response.
    Security,
}

impl BrowserSessionRevocationReason {
    /// Returns the stable database representation.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Logout => "logout",
            Self::Administrative => "administrative",
            Self::Security => "security",
        }
    }
}
