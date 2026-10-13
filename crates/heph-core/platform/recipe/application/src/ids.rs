use std::{fmt, str::FromStr};

use serde::Serialize;
use uuid::Uuid;

use crate::DeploymentError;

macro_rules! identifier {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
        #[serde(transparent)]
        pub struct $name(Uuid);

        impl $name {
            /// Validates a nonnil durable identifier.
            ///
            /// # Errors
            /// Returns an error for a nil identifier.
            pub const fn from_uuid(value: Uuid) -> Result<Self, DeploymentError> {
                if value.is_nil() {
                    Err(DeploymentError::InvalidIdentifier)
                } else {
                    Ok(Self(value))
                }
            }

            /// Returns the stable UUID representation.
            #[must_use]
            pub const fn as_uuid(self) -> Uuid {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }

        impl FromStr for $name {
            type Err = DeploymentError;
            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::from_uuid(
                    Uuid::parse_str(value).map_err(|_| DeploymentError::InvalidIdentifier)?,
                )
            }
        }
    };
}

identifier!(
    DeploymentId,
    "A stable deployment identity independent of request attempts."
);
identifier!(
    DeploymentCommandId,
    "An actor and operation scoped logical command identity."
);
identifier!(
    DeploymentAttemptId,
    "A unique execution or reconciliation attempt identity."
);

/// Maximum UTF-8 byte length of a human-selected deployment key.
pub const MAX_DEPLOYMENT_KEY_BYTES: usize = 128;

/// Validated project-local deployment key; never a provider path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct DeploymentKey(String);

impl DeploymentKey {
    /// Validates exact UTF-8 text without normalizing distinct caller keys.
    ///
    /// # Errors
    /// Rejects empty, overlong, control-containing, or padded keys.
    pub fn parse(value: impl Into<String>) -> Result<Self, DeploymentError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > MAX_DEPLOYMENT_KEY_BYTES
            || value.trim() != value
            || value.chars().any(char::is_control)
        {
            return Err(DeploymentError::InvalidKey);
        }
        Ok(Self(value))
    }

    /// Returns the exact key text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
