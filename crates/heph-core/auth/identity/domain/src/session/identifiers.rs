use std::{fmt, str::FromStr};
use uuid::Uuid;

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

    pub(super) const fn as_uuid(self) -> Uuid {
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
pub struct BrowserSessionDigest(pub(super) [u8; 32]);

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
pub struct BrowserSessionIdentityBindingDigest(pub(super) [u8; 32]);

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
