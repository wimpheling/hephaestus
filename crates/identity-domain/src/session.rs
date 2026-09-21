//! Durable browser-session identity values.
//!
//! The session ID is a bearer value at the browser and mediator boundaries.
//! Its `Debug` and `Display` implementations are deliberately redacted;
//! callers must use [`BrowserSessionSid::to_protocol_string`] only when
//! explicitly serializing the ID for those protocols.

use super::{AuthenticatedIdentity, UserId};
use sha2::{Digest, Sha256};
use std::{fmt, str::FromStr};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

const BROWSER_SESSION_SID_DOMAIN: &[u8] = b"hephaestus-human-browser-session-sid-v1\0";
const BROWSER_SESSION_IDENTITY_DOMAIN: &[u8] = b"hephaestus-human-browser-session-identity-v1\0";

/// Default server-selected browser-session lifetime.
pub const DEFAULT_BROWSER_SESSION_TTL_SECONDS: i64 = 12 * 60 * 60;
/// Database-enforced upper bound for a browser-session lifetime.
pub const MAX_BROWSER_SESSION_TTL_SECONDS: i64 = 24 * 60 * 60;

/// Internal durable row identity for one browser session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BrowserSessionId(Uuid);

impl BrowserSessionId {
    /// Creates a new random row identity.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Restores an identity from its UUID representation.
    #[must_use]
    pub const fn from_uuid(value: Uuid) -> Self {
        Self(value)
    }

    /// Returns the UUID for database binding and event correlation.
    #[must_use]
    pub const fn as_uuid(self) -> Uuid {
        self.0
    }
}

impl Default for BrowserSessionId {
    fn default() -> Self {
        Self::new()
    }
}

/// Random UUID carried by the signed browser cookie and mediator assertion.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct BrowserSessionSid(Uuid);

impl BrowserSessionSid {
    /// Creates a new random session ID.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Restores an ID from its UUID representation.
    #[must_use]
    pub const fn from_uuid(value: Uuid) -> Self {
        Self(value)
    }

    const fn as_uuid(self) -> Uuid {
        self.0
    }

    /// Serializes the ID for an explicit cookie or mediator protocol field.
    #[must_use]
    pub fn to_protocol_string(self) -> String {
        self.0.to_string()
    }
}

impl Default for BrowserSessionSid {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for BrowserSessionSid {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("BrowserSessionSid(REDACTED)")
    }
}

impl fmt::Display for BrowserSessionSid {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[redacted]")
    }
}

impl FromStr for BrowserSessionSid {
    type Err = uuid::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(value).map(Self)
    }
}

/// One-way database verifier for a browser session ID.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct BrowserSessionDigest([u8; 32]);

impl BrowserSessionDigest {
    /// Returns the digest bytes for a bound SQL parameter.
    #[must_use]
    pub const fn as_bytes(self) -> [u8; 32] {
        self.0
    }
}

impl fmt::Debug for BrowserSessionDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("BrowserSessionDigest(REDACTED)")
    }
}

impl fmt::Display for BrowserSessionDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[redacted]")
    }
}

/// One-way binding of a session to the verified issuer and subject.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct BrowserSessionIdentityBindingDigest([u8; 32]);

impl BrowserSessionIdentityBindingDigest {
    /// Returns the digest bytes for a bound SQL parameter.
    #[must_use]
    pub const fn as_bytes(self) -> [u8; 32] {
        self.0
    }
}

impl fmt::Debug for BrowserSessionIdentityBindingDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("BrowserSessionIdentityBindingDigest(REDACTED)")
    }
}

impl fmt::Display for BrowserSessionIdentityBindingDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[redacted]")
    }
}

/// Hashes a session ID with a domain separator before durable storage.
#[must_use]
pub fn browser_session_sid_digest(session_id: BrowserSessionSid) -> BrowserSessionDigest {
    let mut digest = Sha256::new();
    digest.update(BROWSER_SESSION_SID_DOMAIN);
    digest.update(session_id.as_uuid().as_bytes());
    BrowserSessionDigest(digest.finalize().into())
}

/// Binds a session to the exact verified OIDC issuer and subject.
///
/// Each UTF-8 field is length-prefixed before hashing so different issuer and
/// subject pairs cannot become the same byte sequence through concatenation.
#[must_use]
pub fn browser_session_identity_binding_digest(
    identity: &AuthenticatedIdentity,
) -> BrowserSessionIdentityBindingDigest {
    let mut digest = Sha256::new();
    digest.update(BROWSER_SESSION_IDENTITY_DOMAIN);
    update_hash_field(&mut digest, identity.issuer.as_bytes());
    update_hash_field(&mut digest, identity.subject.as_bytes());
    BrowserSessionIdentityBindingDigest(digest.finalize().into())
}

fn update_hash_field(digest: &mut Sha256, value: &[u8]) {
    let length = u64::try_from(value.len()).expect("UTF-8 field length fits in u64");
    digest.update(length.to_be_bytes());
    digest.update(value);
}

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
            || lifetime > Duration::seconds(MAX_BROWSER_SESSION_TTL_SECONDS)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_id_is_redacted_except_for_explicit_protocol_serialization() {
        let id = BrowserSessionSid::new();
        let protocol = id.to_protocol_string();
        assert_eq!(protocol.parse::<BrowserSessionSid>().expect("UUID"), id);
        assert_eq!(id.to_string(), "[redacted]");
        assert!(!format!("{id:?}").contains(&protocol));
        let digest = browser_session_sid_digest(id);
        assert_eq!(digest.to_string(), "[redacted]");
        assert!(!format!("{digest:?}").contains(&protocol));
    }

    #[test]
    fn digest_is_domain_separated_and_stable() {
        let id = BrowserSessionSid::from_uuid(Uuid::from_u128(1));
        let expected = [
            0x31, 0x4e, 0x24, 0x26, 0xd4, 0x0a, 0x7d, 0x68, 0x37, 0xa6, 0x57, 0x45, 0x9f, 0xde,
            0x34, 0x4e, 0x92, 0x37, 0x5e, 0xe2, 0x0a, 0x30, 0x4d, 0x0a, 0x63, 0xea, 0x56, 0x38,
            0xbc, 0xc8, 0xca, 0x1d,
        ];
        assert_eq!(browser_session_sid_digest(id).as_bytes(), expected);
        assert_ne!(
            browser_session_sid_digest(id),
            browser_session_sid_digest(BrowserSessionSid::from_uuid(Uuid::from_u128(2)))
        );
    }

    #[test]
    fn identity_binding_digest_is_stable_and_length_delimited() {
        let identity = super::super::AuthenticatedIdentity::new(
            super::super::UserId::from_uuid(Uuid::from_u128(1)),
            "https://issuer.example",
            "subject",
            serde_json::Value::Null,
            super::super::RequestId::from_uuid(Uuid::from_u128(2)),
        );
        let expected = [
            0xa7, 0x45, 0x55, 0x1d, 0xf9, 0xfe, 0xc2, 0x16, 0x0f, 0x16, 0x54, 0x80, 0x4e, 0xa6,
            0x97, 0x5e, 0x2a, 0xa6, 0x4a, 0xca, 0xa2, 0x91, 0x7d, 0x51, 0x00, 0x98, 0x76, 0x4d,
            0x88, 0x12, 0xb3, 0x17,
        ];
        let digest = browser_session_identity_binding_digest(&identity);
        assert_eq!(digest.as_bytes(), expected);
        assert_eq!(digest.to_string(), "[redacted]");
        assert!(!format!("{digest:?}").contains("issuer.example"));

        let left = super::super::AuthenticatedIdentity::new(
            super::super::UserId::from_uuid(Uuid::from_u128(3)),
            "ab",
            "c",
            serde_json::Value::Null,
            super::super::RequestId::from_uuid(Uuid::from_u128(4)),
        );
        let right = super::super::AuthenticatedIdentity::new(
            super::super::UserId::from_uuid(Uuid::from_u128(3)),
            "a",
            "bc",
            serde_json::Value::Null,
            super::super::RequestId::from_uuid(Uuid::from_u128(4)),
        );
        assert_ne!(
            browser_session_identity_binding_digest(&left),
            browser_session_identity_binding_digest(&right)
        );
    }

    #[test]
    fn metadata_enforces_bounded_lifetime_and_revocation() {
        let issued = OffsetDateTime::UNIX_EPOCH;
        let expires = issued + Duration::seconds(DEFAULT_BROWSER_SESSION_TTL_SECONDS);
        let metadata = BrowserSessionMetadata::new(
            BrowserSessionId::new(),
            UserId::new(),
            issued,
            expires,
            None,
        )
        .expect("valid metadata");
        assert!(!metadata.is_active_at(issued - Duration::seconds(1)));
        assert!(metadata.is_active_at(issued));
        assert!(metadata.is_active_at(issued + Duration::hours(1)));
        assert!(!metadata.is_active_at(expires));
        assert!(
            BrowserSessionMetadata::new(
                metadata.id(),
                metadata.user_id(),
                issued,
                issued + Duration::seconds(MAX_BROWSER_SESSION_TTL_SECONDS),
                None,
            )
            .is_some()
        );
        assert!(
            BrowserSessionMetadata::new(
                metadata.id(),
                metadata.user_id(),
                issued,
                issued + Duration::seconds(MAX_BROWSER_SESSION_TTL_SECONDS + 1),
                None,
            )
            .is_none()
        );
        assert!(
            BrowserSessionMetadata::new(metadata.id(), metadata.user_id(), issued, issued, None,)
                .is_none()
        );
        assert!(
            BrowserSessionMetadata::new(
                metadata.id(),
                metadata.user_id(),
                issued,
                expires,
                Some(issued - Duration::seconds(1)),
            )
            .is_none()
        );
        let revoked = BrowserSessionMetadata::new(
            metadata.id(),
            metadata.user_id(),
            issued,
            expires,
            Some(issued),
        )
        .expect("valid revoked metadata");
        assert!(!revoked.is_active_at(issued));
    }
}
