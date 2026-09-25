use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};
use uuid::Uuid;

use crate::errors::GitCapabilityError;

/// A canonical opaque repository identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RepositoryId(Uuid);

impl RepositoryId {
    /// Creates an identifier from a UUID.
    #[must_use]
    pub const fn new(value: Uuid) -> Self {
        Self(value)
    }

    /// Returns the underlying UUID.
    #[must_use]
    pub const fn as_uuid(self) -> Uuid {
        self.0
    }
}

impl FromStr for RepositoryId {
    type Err = GitCapabilityError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let parsed = Uuid::parse_str(value)
            .map_err(|_| GitCapabilityError::NonCanonicalRepositoryId(value.to_owned()))?;
        if parsed.hyphenated().to_string() != value {
            return Err(GitCapabilityError::NonCanonicalRepositoryId(
                value.to_owned(),
            ));
        }
        Ok(Self(parsed))
    }
}

impl fmt::Display for RepositoryId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.hyphenated().fmt(formatter)
    }
}

/// A smart-HTTP Git operation authorized by a capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GitOperation {
    /// Discover visible refs through upload-pack advertisement.
    Discover,
    /// Fetch objects through upload-pack.
    Fetch,
    /// Propose atomic ref changes through receive-pack.
    Receive,
}
