//! Typed authorization values and transaction-aware provider contract.

use async_trait::async_trait;
use identity_domain::UserId;
use serde::{Deserialize, Serialize};
use sqlx::{Postgres, Transaction};
use std::{fmt, str::FromStr};
use uuid::Uuid;

/// An authorization subject.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Subject {
    /// One authenticated internal user.
    User(UserId),
}

impl Subject {
    /// Returns the compiler subject type.
    #[must_use]
    pub const fn object_type(self) -> &'static str {
        match self {
            Self::User(_) => "user",
        }
    }

    /// Returns the stable textual subject identifier.
    #[must_use]
    pub fn id(self) -> String {
        match self {
            Self::User(id) => id.to_string(),
        }
    }
}

/// Resource types represented by the canonical authorization model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectType {
    /// Organization.
    Organization,
    /// Project.
    Project,
    /// Git repository.
    Repository,
    /// Configured agent.
    Agent,
    /// Agent execution.
    Run,
    /// Persistent agent-state volume.
    StateVolume,
}

impl ObjectType {
    /// Returns the canonical model name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Organization => "organization",
            Self::Project => "project",
            Self::Repository => "repository",
            Self::Agent => "agent",
            Self::Run => "run",
            Self::StateVolume => "state_volume",
        }
    }
}

impl FromStr for ObjectType {
    type Err = AuthzError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "organization" => Ok(Self::Organization),
            "project" => Ok(Self::Project),
            "repository" => Ok(Self::Repository),
            "agent" => Ok(Self::Agent),
            "run" => Ok(Self::Run),
            "state_volume" => Ok(Self::StateVolume),
            _ => Err(AuthzError::UnknownObjectType(value.to_owned())),
        }
    }
}

/// A typed resource reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectRef {
    /// Resource type.
    pub object_type: ObjectType,
    /// Stable resource UUID.
    pub id: Uuid,
}

impl ObjectRef {
    /// Creates a resource reference.
    #[must_use]
    pub const fn new(object_type: ObjectType, id: Uuid) -> Self {
        Self { object_type, id }
    }
}

impl fmt::Display for ObjectRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}:{}", self.object_type.as_str(), self.id)
    }
}

/// Permissions exposed by the application authorization boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Permission {
    /// Read a resource.
    CanRead,
    /// Write a resource.
    CanWrite,
    /// Manage a resource.
    CanManage,
    /// Delete a resource.
    CanDelete,
    /// Manage organization membership.
    CanManageMembers,
    /// Create a project.
    CanCreateProject,
    /// Execute an agent.
    CanExecute,
    /// Cancel a run.
    CanCancel,
    /// Attach a state volume.
    CanAttach,
    /// Restore a state volume.
    CanRestore,
}

impl Permission {
    /// Returns the canonical relation name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CanRead => "can_read",
            Self::CanWrite => "can_write",
            Self::CanManage => "can_manage",
            Self::CanDelete => "can_delete",
            Self::CanManageMembers => "can_manage_members",
            Self::CanCreateProject => "can_create_project",
            Self::CanExecute => "can_execute",
            Self::CanCancel => "can_cancel",
            Self::CanAttach => "can_attach",
            Self::CanRestore => "can_restore",
        }
    }
}

impl FromStr for Permission {
    type Err = AuthzError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "can_read" => Ok(Self::CanRead),
            "can_write" => Ok(Self::CanWrite),
            "can_manage" => Ok(Self::CanManage),
            "can_delete" => Ok(Self::CanDelete),
            "can_manage_members" => Ok(Self::CanManageMembers),
            "can_create_project" => Ok(Self::CanCreateProject),
            "can_execute" => Ok(Self::CanExecute),
            "can_cancel" => Ok(Self::CanCancel),
            "can_attach" => Ok(Self::CanAttach),
            "can_restore" => Ok(Self::CanRestore),
            _ => Err(AuthzError::UnknownPermission(value.to_owned())),
        }
    }
}

/// Result returned by an authorization provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuthorizationDecision {
    /// The operation is permitted.
    Allow,
    /// The operation is denied.
    Deny,
}

impl AuthorizationDecision {
    /// Returns whether the decision permits the operation.
    #[must_use]
    pub const fn is_allowed(self) -> bool {
        matches!(self, Self::Allow)
    }
}

/// Typed authorization failure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum AuthzError {
    /// Actor context was not set on the transaction.
    #[error("authenticated actor context is missing")]
    MissingActorContext,
    /// An object type was not recognized.
    #[error("unknown authorization object type {0:?}")]
    UnknownObjectType(String),
    /// A permission was not recognized.
    #[error("unknown authorization permission {0:?}")]
    UnknownPermission(String),
    /// A subject or object identifier was malformed.
    #[error("malformed authorization identifier {0:?}")]
    MalformedId(String),
    /// The database authorization evaluator failed.
    #[error("authorization evaluator failed: {0}")]
    Database(#[source] sqlx::Error),
}

/// Provider-neutral transaction-aware authorization boundary.
#[async_trait]
pub trait Authorizer: Send + Sync {
    /// Checks one permission using current-transaction domain state.
    ///
    /// # Errors
    ///
    /// Returns a typed error for missing context, invalid values, or evaluator
    /// failure. Invalid inputs never produce an allow decision.
    async fn check(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        subject: Subject,
        permission: Permission,
        object: ObjectRef,
    ) -> Result<AuthorizationDecision, AuthzError>;
}

#[cfg(test)]
mod tests {
    use super::{ObjectType, Permission};
    use std::str::FromStr;

    #[test]
    fn validates_model_names() {
        assert_eq!(
            ObjectType::from_str("state_volume").expect("known type"),
            ObjectType::StateVolume
        );
        assert!(ObjectType::from_str("unknown").is_err());
        assert_eq!(
            Permission::from_str("can_execute").expect("known permission"),
            Permission::CanExecute
        );
        assert!(Permission::from_str("owner").is_err());
    }
}
