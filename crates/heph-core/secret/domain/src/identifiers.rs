use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};
use uuid::Uuid;

use super::SecretValueError;

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

identifier!(SecretId, "A stable identifier for an owned secret.");
identifier!(
    SecretVersionId,
    "A stable identifier for an immutable encrypted secret version."
);
identifier!(
    SecretGrantId,
    "A stable identifier for an explicit source-side delegation grant."
);
identifier!(
    SecretImportId,
    "A stable identifier for an accepted opaque secret import."
);
identifier!(
    AgentSecretBindingId,
    "A stable identifier for an immutable agent-revision secret binding."
);
identifier!(
    SecretLeaseId,
    "A stable identifier for short-lived runtime secret authority."
);
identifier!(
    SecretRuntimeSessionId,
    "A stable identifier for one authenticated runtime secret session."
);
identifier!(
    GatewaySecretBindingId,
    "A stable identifier for an immutable gateway-revision secret binding."
);
identifier!(
    GatewaySecretLeaseId,
    "A stable identifier for one gateway invocation secret lease."
);

macro_rules! bounded_key {
    ($name:ident, $description:literal, $minimum:expr, $maximum:expr) => {
        #[doc = $description]
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            /// Parses and validates a key.
            ///
            /// # Errors
            ///
            /// Returns [`SecretValueError`] if the value is not a bounded,
            /// lowercase identifier.
            pub fn parse(value: impl Into<String>) -> Result<Self, SecretValueError> {
                let value = value.into();
                let valid = ($minimum..=$maximum).contains(&value.len())
                    && value.bytes().enumerate().all(|(index, byte)| {
                        byte.is_ascii_lowercase()
                            || byte.is_ascii_digit()
                            || ((byte == b'_' || byte == b'-') && index > 0)
                    });
                if !valid {
                    return Err(SecretValueError::InvalidKey {
                        kind: stringify!($name),
                    });
                }
                Ok(Self(value))
            }

            /// Returns the validated key.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }

        impl TryFrom<String> for $name {
            type Error = SecretValueError;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::parse(value)
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.0
            }
        }
    };
}

bounded_key!(SecretName, "A name unique within one secret owner.", 1, 128);
bounded_key!(
    SecretAlias,
    "A target-local name for an opaque secret import.",
    1,
    128
);
bounded_key!(
    SecretSlotKey,
    "A symbolic secret capability declared by a release.",
    1,
    64
);
