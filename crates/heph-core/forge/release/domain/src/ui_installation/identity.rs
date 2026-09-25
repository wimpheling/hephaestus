use crate::{ReleaseCommandKey, ReleaseValueError};
use forge_domain::{OrganizationId, ProjectId, RepositoryId};
use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

/// Maximum UTF-8 byte length of a caller-supplied idempotency key.
pub const MAX_UI_INSTALLATION_CALLER_KEY_BYTES: usize = 256;

/// A bounded opaque caller idempotency key.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct UiInstallationCallerKey(String);

impl UiInstallationCallerKey {
    /// Parses a bounded caller key while preserving its bytes.
    ///
    /// # Errors
    ///
    /// Returns an invalid-key error for empty, NUL-containing, or oversized
    /// input. Other whitespace and control bytes remain caller data.
    pub fn parse(value: impl Into<String>) -> Result<Self, ReleaseValueError> {
        let value = value.into();
        let valid = (1..=MAX_UI_INSTALLATION_CALLER_KEY_BYTES).contains(&value.len())
            && !value.as_bytes().contains(&0);
        if valid {
            Ok(Self(value))
        } else {
            Err(ReleaseValueError::InvalidKey {
                kind: "ui installation caller idempotency key",
            })
        }
    }

    /// Returns the validated caller key.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for UiInstallationCallerKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl TryFrom<String> for UiInstallationCallerKey {
    type Error = ReleaseValueError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<UiInstallationCallerKey> for String {
    fn from(value: UiInstallationCallerKey) -> Self {
        value.0
    }
}

/// Navigation owner for an installation. Global installations are owned by
/// an organization; personal global installations are intentionally unsupported.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "scope", content = "id", rename_all = "snake_case")]
pub enum UiInstallationTarget {
    /// A project-wide installation.
    Project(ProjectId),
    /// An installation scoped to one repository.
    Repository(RepositoryId),
    /// An organization-owned installation visible across that organization.
    #[serde(rename = "global")]
    Organization(OrganizationId),
}

impl UiInstallationTarget {
    /// Creates a project-scoped target.
    #[must_use]
    pub const fn project(project_id: ProjectId) -> Self {
        Self::Project(project_id)
    }

    /// Creates a repository-scoped target.
    #[must_use]
    pub const fn repository(repository_id: RepositoryId) -> Self {
        Self::Repository(repository_id)
    }

    /// Creates an organization-owned global target.
    #[must_use]
    pub const fn organization(organization_id: OrganizationId) -> Self {
        Self::Organization(organization_id)
    }

    /// Returns the target UUID.
    #[must_use]
    pub const fn as_uuid(self) -> Uuid {
        match self {
            Self::Project(id) => id.as_uuid(),
            Self::Repository(id) => id.as_uuid(),
            Self::Organization(id) => id.as_uuid(),
        }
    }

    /// Returns the stable scope discriminator used by canonical hashing.
    #[must_use]
    pub const fn scope_name(self) -> &'static str {
        match self {
            Self::Project(_) => "project",
            Self::Repository(_) => "repository",
            Self::Organization(_) => "global",
        }
    }
}

/// Lifecycle of a stable installation identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UiInstallationState {
    /// The current generation may be projected and served.
    Enabled,
    /// The installation remains retained but is unavailable for projection.
    Disabled,
    /// The installation is terminal and its key may be reused by a new identity.
    Removed,
}

impl UiInstallationState {
    /// Returns whether an installation may move to the next state.
    #[must_use]
    pub const fn can_transition_to(self, next: Self) -> bool {
        matches!(
            (self, next),
            (
                Self::Enabled | Self::Disabled,
                Self::Enabled | Self::Disabled | Self::Removed
            )
        )
    }
}

/// Installation command operation used for actor-bound idempotency.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UiInstallationOperation {
    /// Create a stable installation identity and its first generation.
    Install,
    /// Create a fresh generation or reactivate an installation.
    Activate,
    /// Roll back to a selected published release as a fresh generation.
    Rollback,
    /// Disable an installation without deleting its history.
    Disable,
    /// Terminally remove an installation.
    Remove,
}

impl UiInstallationOperation {
    /// Returns the canonical operation spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Install => "install",
            Self::Activate => "activate",
            Self::Rollback => "rollback",
            Self::Disable => "disable",
            Self::Remove => "remove",
        }
    }
}

/// Actor, operation, and caller key that identify one replayable command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiInstallationCommandIdentity {
    actor_id: Uuid,
    operation: UiInstallationOperation,
    caller_key: UiInstallationCallerKey,
}

impl UiInstallationCommandIdentity {
    /// Creates an actor-bound command identity.
    #[must_use]
    pub const fn new(
        actor_id: Uuid,
        operation: UiInstallationOperation,
        caller_key: UiInstallationCallerKey,
    ) -> Self {
        Self {
            actor_id,
            operation,
            caller_key,
        }
    }

    /// Returns the derived actor-bound command key.
    #[must_use]
    pub fn command_key(&self) -> ReleaseCommandKey {
        ReleaseCommandKey::derive(
            "ui-installation-command-v1",
            &[
                self.operation.as_str().as_bytes(),
                self.actor_id.as_bytes(),
                self.caller_key.as_str().as_bytes(),
            ],
        )
    }
}
