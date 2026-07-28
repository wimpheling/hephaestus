//! Provider-neutral forge identifiers and durable domain values.

use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};
use time::OffsetDateTime;
use uuid::Uuid;

pub use identity_domain::OrganizationId;

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

identifier!(ProjectId, "A stable opaque project identifier.");
identifier!(RepositoryId, "A stable opaque repository identifier.");
identifier!(
    ReceiveId,
    "A stable identifier for one Git receive transaction."
);
identifier!(
    AgentConfigRevisionId,
    "A stable identifier for one parsed agent configuration revision."
);
identifier!(
    RunRequestId,
    "A stable identifier for one forge-originated run request."
);

/// A validated fully-qualified Git reference name.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct GitRef(String);

impl GitRef {
    /// Parses and validates a fully-qualified reference name.
    ///
    /// # Errors
    ///
    /// Returns [`GitValueError`] when the value violates Git reference naming
    /// constraints or is not beneath `refs/`.
    pub fn parse(value: impl Into<String>) -> Result<Self, GitValueError> {
        let value = value.into();
        let invalid_character = value
            .bytes()
            .any(|byte| byte <= b' ' || byte == 0x7f || b"~^:?*[\\".contains(&byte));
        if !value.starts_with("refs/")
            || value.len() <= "refs/".len()
            || value.ends_with('/')
            || value.ends_with('.')
            || value.contains("..")
            || value.contains("@{")
            || value.contains("//")
            || value.split('/').any(|part| {
                part.is_empty()
                    || part.starts_with('.')
                    || part
                        .get(part.len().saturating_sub(5)..)
                        .is_some_and(|suffix| suffix.eq_ignore_ascii_case(".lock"))
                    || part.ends_with('.')
            })
            || invalid_character
        {
            return Err(GitValueError::InvalidRef(value));
        }
        Ok(Self(value))
    }

    /// Returns the validated reference name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for GitRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl TryFrom<String> for GitRef {
    type Error = GitValueError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<GitRef> for String {
    fn from(value: GitRef) -> Self {
        value.0
    }
}

/// A validated hexadecimal Git object identifier.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct CommitSha(String);

impl CommitSha {
    /// Parses a full SHA-1 or SHA-256 object identifier.
    ///
    /// # Errors
    ///
    /// Returns [`GitValueError`] for abbreviated, uppercase, or non-hex values.
    pub fn parse(value: impl Into<String>) -> Result<Self, GitValueError> {
        let value = value.into();
        if !matches!(value.len(), 40 | 64)
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(GitValueError::InvalidCommit(value));
        }
        Ok(Self(value))
    }

    /// Returns the normalized hexadecimal object identifier.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for CommitSha {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl TryFrom<String> for CommitSha {
    type Error = GitValueError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<CommitSha> for String {
    fn from(value: CommitSha) -> Self {
        value.0
    }
}

/// Validation failure for a Git domain value.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum GitValueError {
    /// Invalid reference name.
    #[error("invalid fully-qualified Git reference {0:?}")]
    InvalidRef(String),
    /// Invalid full object identifier.
    #[error("invalid Git commit identifier {0:?}")]
    InvalidCommit(String),
}

/// Durable project metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    /// Stable project identifier.
    pub id: ProjectId,
    /// Owning organization.
    pub organization_id: OrganizationId,
    /// Human-readable project name.
    pub name: String,
    /// Creation time.
    pub created_at: OffsetDateTime,
}

/// Durable repository metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repository {
    /// Stable repository identifier.
    pub id: RepositoryId,
    /// Owning project.
    pub project_id: ProjectId,
    /// Human-readable repository name.
    pub name: String,
    /// Default fully-qualified branch ref.
    pub default_branch: GitRef,
    /// Whether unaffiliated users may read the repository.
    pub is_public: bool,
    /// Whether pushes may trigger agent runs.
    pub agent_runs_enabled: bool,
    /// Creation time.
    pub created_at: OffsetDateTime,
}

/// One accepted update in a receive transaction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefUpdate {
    /// Updated reference.
    pub git_ref: GitRef,
    /// Previous object identifier, absent when the ref was created.
    pub old_commit: Option<CommitSha>,
    /// New object identifier, absent when the ref was deleted.
    pub new_commit: Option<CommitSha>,
}

#[cfg(test)]
mod tests {
    use super::{CommitSha, GitRef};

    #[test]
    fn validates_git_values() {
        assert!(GitRef::parse("refs/heads/main").is_ok());
        assert!(GitRef::parse("refs/heads/../secret").is_err());
        assert!(GitRef::parse("../../refs/heads/main").is_err());
        assert!(CommitSha::parse("a".repeat(40)).is_ok());
        assert!(CommitSha::parse("A".repeat(40)).is_err());
        assert!(CommitSha::parse("a".repeat(12)).is_err());
    }
}
