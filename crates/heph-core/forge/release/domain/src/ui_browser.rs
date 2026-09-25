//! Domain primitives for one-time UI handoffs and generation-bound browser
//! sessions.
//!
//! This module contains no persistence or transport behavior. Raw bearer
//! secrets are intentionally neither serializable nor displayable; adapters
//! hash them before storage and place them only at their explicit transport
//! boundaries.

use crate::ui::UiRoutePath;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

/// Lifetime of a one-time browser handoff.
pub const UI_BROWSER_HANDOFF_TTL_SECONDS: i64 = 60;
/// Maximum lifetime of a child UI browser session from its issue instant.
pub const UI_BROWSER_SESSION_TTL_SECONDS: i64 = 12 * 60 * 60;
const HANDOFF_SECRET_DOMAIN: &[u8] = b"hephaestus-ui-browser-handoff-v1\0";
const SESSION_SECRET_DOMAIN: &[u8] = b"hephaestus-ui-browser-session-v1\0";

macro_rules! browser_identifier {
    ($name:ident, $documentation:literal) => {
        #[doc = $documentation]
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(Uuid);

        impl $name {
            /// Creates a random internal row identity.
            #[must_use]
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }

            /// Restores an identity read from durable storage.
            #[must_use]
            pub const fn from_uuid(value: Uuid) -> Self {
                Self(value)
            }

            /// Returns the UUID for database binding and internal correlation.
            #[must_use]
            pub const fn as_uuid(self) -> Uuid {
                self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }
    };
}

browser_identifier!(
    UiBrowserHandoffId,
    "Internal row identity for one one-time UI browser handoff."
);
browser_identifier!(
    UiBrowserSessionId,
    "Internal row identity for one generation-bound UI browser session."
);

macro_rules! bearer_secret {
    (
        $secret:ident,
        $digest_type:ident,
        $domain:ident,
        $secret_doc:literal,
        $digest_doc:literal
    ) => {
        #[doc = $secret_doc]
        pub struct $secret([u8; 32]);

        impl $secret {
            /// Generates a 32-byte bearer value from two platform UUID-v4
            /// values. UUID-v4 encodes 244 bits of entropy in this 32-byte
            /// representation.
            #[must_use]
            pub fn random() -> Self {
                let first = Uuid::new_v4();
                let second = Uuid::new_v4();
                let mut bytes = [0; 32];
                bytes[..16].copy_from_slice(first.as_bytes());
                bytes[16..].copy_from_slice(second.as_bytes());
                Self(bytes)
            }

            /// Restores a secret supplied at its explicit transport boundary.
            /// Callers must not log or serialize the returned value.
            #[must_use]
            pub const fn from_bytes(bytes: [u8; 32]) -> Self {
                Self(bytes)
            }

            /// Returns the raw bytes for this secret's explicit transport
            /// boundary: a sensitive Phoenix request for a handoff, or an
            /// explicit `Set-Cookie` for a child session.
            #[must_use]
            pub const fn as_bytes(&self) -> &[u8; 32] {
                &self.0
            }

            /// Computes the domain-separated one-way storage digest.
            #[must_use]
            pub fn digest(&self) -> $digest_type {
                let mut digest = Sha256::new();
                digest.update($domain);
                digest.update(self.0);
                $digest_type(digest.finalize().into())
            }
        }

        impl fmt::Debug for $secret {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(concat!(stringify!($secret), "(REDACTED)"))
            }
        }

        #[doc = $digest_doc]
        #[derive(Clone, Copy, PartialEq, Eq, Hash)]
        pub struct $digest_type([u8; 32]);

        impl $digest_type {
            /// Returns digest bytes for a parameterized persistence adapter.
            #[must_use]
            pub const fn as_bytes(self) -> [u8; 32] {
                self.0
            }
        }

        impl fmt::Debug for $digest_type {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(concat!(stringify!($digest_type), "(REDACTED)"))
            }
        }

        impl fmt::Display for $digest_type {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("[redacted]")
            }
        }
    };
}

bearer_secret!(
    UiBrowserHandoffSecret,
    UiBrowserHandoffDigest,
    HANDOFF_SECRET_DOMAIN,
    "Raw one-time handoff bearer secret. It has no `Display` or `Serialize` implementation.",
    "One-way digest of a UI handoff secret."
);
bearer_secret!(
    UiBrowserSessionSecret,
    UiBrowserSessionDigest,
    SESSION_SECRET_DOMAIN,
    "Raw child-session bearer secret. It has no `Display` or `Serialize` implementation.",
    "One-way digest of a UI child-session secret."
);

/// A validated platform-relative route selected by the UI declaration.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct UiBrowserRoute(UiRoutePath);

impl UiBrowserRoute {
    /// Parses a route through the release UI path validator. This accepts no
    /// URL scheme, authority, query, fragment, absolute path, or traversal.
    ///
    /// # Errors
    ///
    /// Returns [`UiBrowserRouteError::Invalid`] for any non-relative route.
    pub fn parse(value: impl Into<String>) -> Result<Self, UiBrowserRouteError> {
        UiRoutePath::parse(value)
            .map(Self)
            .map_err(|_| UiBrowserRouteError::Invalid)
    }

    /// Returns the normalized validated path for routing within the bound UI.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Returns the underlying release-domain typed path.
    #[must_use]
    pub const fn path(&self) -> &UiRoutePath {
        &self.0
    }
}

impl TryFrom<String> for UiBrowserRoute {
    type Error = UiBrowserRouteError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<UiBrowserRoute> for String {
    fn from(value: UiBrowserRoute) -> Self {
        value.0.into()
    }
}

impl fmt::Display for UiBrowserRoute {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// The route was not a safe platform-relative UI path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum UiBrowserRouteError {
    /// The route contained URL syntax, traversal, or invalid bytes.
    #[error("UI browser route is invalid")]
    Invalid,
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bearer_digests_are_stable_domain_separated_and_redacted() {
        let bytes = [0x5a; 32];
        let handoff = UiBrowserHandoffSecret::from_bytes(bytes);
        let session = UiBrowserSessionSecret::from_bytes(bytes);
        let handoff_digest = handoff.digest();
        let session_digest = session.digest();

        assert_eq!(handoff_digest, handoff.digest());
        assert_ne!(handoff_digest.as_bytes(), session_digest.as_bytes());
        assert_eq!(format!("{handoff:?}"), "UiBrowserHandoffSecret(REDACTED)");
        assert_eq!(
            format!("{handoff_digest:?}"),
            "UiBrowserHandoffDigest(REDACTED)"
        );
        assert_eq!(format!("{session_digest}"), "[redacted]");
        assert!(!format!("{handoff:?}").contains("5a"));
    }

    #[test]
    fn child_expiry_is_capped_without_sliding_renewal() {
        let issued = OffsetDateTime::UNIX_EPOCH;
        let parent_later = issued + Duration::hours(24);
        assert_eq!(
            child_session_expiry(issued, parent_later).expect("valid parent"),
            issued + Duration::hours(12)
        );
        let parent_earlier = issued + Duration::hours(2);
        assert_eq!(
            child_session_expiry(issued, parent_earlier).expect("valid parent"),
            parent_earlier
        );
        assert_eq!(
            child_session_expiry(issued, issued),
            Err(UiBrowserSessionFailure::Unauthenticated)
        );

        let maximum =
            OffsetDateTime::new_in_offset(time::Date::MAX, time::Time::MAX, time::UtcOffset::UTC);
        let near_maximum = maximum
            .checked_sub(Duration::hours(1))
            .expect("representable test instant");
        assert_eq!(
            child_session_expiry(near_maximum, maximum),
            Err(UiBrowserSessionFailure::InvalidLifetime)
        );
    }

    #[test]
    fn browser_route_rejects_urls_and_round_trips_only_typed_paths() {
        let route = UiBrowserRoute::parse("assets/app/main.js").expect("safe route");
        assert_eq!(route.as_str(), "assets/app/main.js");
        let encoded = serde_json::to_string(&route).expect("route JSON");
        assert_eq!(
            serde_json::from_str::<UiBrowserRoute>(&encoded).expect("typed route JSON"),
            route
        );
        for value in [
            "",
            "/absolute",
            "//other.example",
            "https://other.example/app",
            "../escape",
            "app?next=other",
            "app#fragment",
        ] {
            assert_eq!(
                UiBrowserRoute::parse(value),
                Err(UiBrowserRouteError::Invalid)
            );
            let json = format!("{value:?}");
            assert!(
                serde_json::from_str::<UiBrowserRoute>(&json).is_err(),
                "{value}"
            );
        }
    }
}
