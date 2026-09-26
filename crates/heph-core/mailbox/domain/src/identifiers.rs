use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{
    MAX_DEDUPLICATION_KEY_BYTES, MAX_METHOD_BYTES, MAX_PRODUCER_KEY_BYTES,
    errors::MailboxDomainError,
};

macro_rules! identifier {
    ($name:ident, $documentation:literal) => {
        #[doc = $documentation]
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(Uuid);

        impl $name {
            /// Creates a random version 4 identifier.
            #[must_use]
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }

            /// Creates an identifier from its UUID representation.
            #[must_use]
            pub const fn from_uuid(value: Uuid) -> Self {
                Self(value)
            }

            /// Returns the UUID representation.
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

        impl FromStr for $name {
            type Err = uuid::Error;
            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Uuid::parse_str(value).map(Self)
            }
        }
    };
}

identifier!(
    MailboxId,
    "A stable identifier for one agent-instance-owned mailbox."
);
identifier!(
    MailboxEventId,
    "A stable identifier for one accepted mailbox event."
);
identifier!(
    DeliveryAttemptId,
    "A stable identifier for one logical delivery attempt."
);
identifier!(
    BodyReferenceId,
    "An opaque identifier for one accepted mailbox body."
);
identifier!(
    MailboxOperationId,
    "A deterministic idempotency identity for one mailbox operation."
);

/// A stable, normalized producer identity within a mailbox's declared scope.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ProducerId(String);

impl ProducerId {
    /// Parses a bounded, printable opaque producer key.
    ///
    /// # Errors
    ///
    /// Returns [`MailboxDomainError::InvalidProducer`] for empty, oversized,
    /// control-character, or surrounding-whitespace values.
    pub fn parse(value: impl Into<String>) -> Result<Self, MailboxDomainError> {
        let value = value.into();
        validate_opaque(
            &value,
            MAX_PRODUCER_KEY_BYTES,
            MailboxDomainError::InvalidProducer,
        )?;
        Ok(Self(value))
    }

    /// Returns the validated opaque key.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ProducerId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}
impl TryFrom<String> for ProducerId {
    type Error = MailboxDomainError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}
impl From<ProducerId> for String {
    fn from(value: ProducerId) -> Self {
        value.0
    }
}

/// An opaque stable deduplication key selected by a producer.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct DeduplicationKey(String);

impl DeduplicationKey {
    /// Parses a bounded opaque idempotency key.
    ///
    /// # Errors
    ///
    /// Returns [`MailboxDomainError::InvalidDeduplicationKey`] for malformed input.
    pub fn parse(value: impl Into<String>) -> Result<Self, MailboxDomainError> {
        let value = value.into();
        validate_opaque(
            &value,
            MAX_DEDUPLICATION_KEY_BYTES,
            MailboxDomainError::InvalidDeduplicationKey,
        )?;
        Ok(Self(value))
    }

    /// Returns the validated opaque key.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for DeduplicationKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}
impl TryFrom<String> for DeduplicationKey {
    type Error = MailboxDomainError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}
impl From<DeduplicationKey> for String {
    fn from(value: DeduplicationKey) -> Self {
        value.0
    }
}
/// A normalized upper-case HTTP-like method carried by an opaque envelope.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct EnvelopeMethod(String);

impl EnvelopeMethod {
    /// Parses an upper-case token method.
    ///
    /// # Errors
    ///
    /// Returns [`MailboxDomainError::InvalidMethod`] for malformed input.
    pub fn parse(value: impl Into<String>) -> Result<Self, MailboxDomainError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > MAX_METHOD_BYTES
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
        {
            return Err(MailboxDomainError::InvalidMethod);
        }
        Ok(Self(value))
    }

    /// Returns the normalized method.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for EnvelopeMethod {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}
impl TryFrom<String> for EnvelopeMethod {
    type Error = MailboxDomainError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}
impl From<EnvelopeMethod> for String {
    fn from(value: EnvelopeMethod) -> Self {
        value.0
    }
}

pub fn validate_opaque(
    value: &str,
    maximum: usize,
    error: MailboxDomainError,
) -> Result<(), MailboxDomainError> {
    if value.is_empty()
        || value.len() > maximum
        || value.trim() != value
        || value.bytes().any(|byte| byte.is_ascii_control())
    {
        return Err(error);
    }
    Ok(())
}
