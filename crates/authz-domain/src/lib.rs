//! Typed authorization values and transaction-aware provider contract.

use async_trait::async_trait;
use identity_domain::AuthenticatedIdentity;
use identity_domain::UserId;
use runtime_types::RunId;
use serde::{Deserialize, Serialize};
use std::{error::Error, fmt, str::FromStr};
use uuid::Uuid;

/// Git operation requiring repository authorization.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitRepositoryOperation {
    /// Read refs and objects.
    Read,
    /// Write refs and objects.
    Write,
}

/// Provider-neutral authorization port for Git transport adapters.
#[async_trait]
pub trait GitRepositoryAuthorizer: Send + Sync + 'static {
    /// Authorizes an authenticated identity for one repository operation.
    async fn authorize_git(
        &self,
        repository_id: Uuid,
        operation: GitRepositoryOperation,
        identity: &AuthenticatedIdentity,
    ) -> Result<AuthorizationDecision, AuthzError>;
}

/// An authorization subject.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Subject {
    /// One authenticated internal user.
    User(UserId),
    /// One authenticated exact runtime run.
    Run(RunId),
}

impl Subject {
    /// Returns the compiler subject type.
    #[must_use]
    pub const fn object_type(self) -> &'static str {
        match self {
            Self::User(_) => "user",
            Self::Run(_) => "run",
        }
    }

    /// Returns the stable textual subject identifier.
    #[must_use]
    pub fn id(self) -> String {
        match self {
            Self::User(id) => id.to_string(),
            Self::Run(id) => id.to_string(),
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
    /// Agent execution.
    Run,
    /// Persistent agent-state volume.
    StateVolume,
    /// Isolated build request.
    Build,
    /// Immutable reusable release.
    Release,
    /// One exported agent in a release.
    ReleaseAgent,
    /// Project-owned reusable agent instance.
    AgentInstance,
    /// Repository/ref attachment.
    AgentAttachment,
    /// Agent instance update.
    AgentUpdate,
    /// Owned secret metadata.
    Secret,
    /// Source-side secret grant.
    SecretGrant,
    /// Target-side opaque secret import.
    SecretImport,
    /// Immutable agent secret binding.
    AgentSecretBinding,
    /// Exact runtime secret lease.
    SecretLease,
}

impl ObjectType {
    /// Returns the canonical model name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Organization => "organization",
            Self::Project => "project",
            Self::Repository => "repository",
            Self::Run => "run",
            Self::StateVolume => "state_volume",
            Self::Build => "build",
            Self::Release => "release",
            Self::ReleaseAgent => "release_agent",
            Self::AgentInstance => "agent_instance",
            Self::AgentAttachment => "agent_attachment",
            Self::AgentUpdate => "agent_update",
            Self::Secret => "secret",
            Self::SecretGrant => "secret_grant",
            Self::SecretImport => "secret_import",
            Self::AgentSecretBinding => "agent_secret_binding",
            Self::SecretLease => "secret_lease",
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
            "run" => Ok(Self::Run),
            "state_volume" => Ok(Self::StateVolume),
            "build" => Ok(Self::Build),
            "release" => Ok(Self::Release),
            "release_agent" => Ok(Self::ReleaseAgent),
            "agent_instance" => Ok(Self::AgentInstance),
            "agent_attachment" => Ok(Self::AgentAttachment),
            "agent_update" => Ok(Self::AgentUpdate),
            "secret" => Ok(Self::Secret),
            "secret_grant" => Ok(Self::SecretGrant),
            "secret_import" => Ok(Self::SecretImport),
            "agent_secret_binding" => Ok(Self::AgentSecretBinding),
            "secret_lease" => Ok(Self::SecretLease),
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
    /// Publish an immutable release.
    CanPublish,
    /// Revoke a published release while retaining its provenance.
    CanRevoke,
    /// Use a published release for a new guest.
    CanUse,
    /// Start an instance update.
    CanUpdate,
    /// Recover a paused instance/update.
    CanRecover,
    /// Inspect secret metadata without retrieving plaintext.
    InspectMetadata,
    /// Submit a secret value without reading the prior value.
    WriteValue,
    /// Rotate a secret to a new immutable version.
    Rotate,
    /// Manage exact target grants.
    ManageGrants,
    /// Revoke secret authority.
    Revoke,
    /// Purge retained encrypted material.
    Purge,
    /// Accept an exact source grant as an opaque import.
    Accept,
    /// Bind or use broker-only authority.
    BindBrokered,
    /// Bind raw guest delivery authority.
    BindRaw,
    /// Use an exact brokered runtime lease.
    UseBrokered,
    /// Receive raw material for an exact runtime lease.
    ReceiveRaw,
    /// Create a secret by submitting its initial value to an owner.
    CanWriteSecretValue,
    /// Assign exact source-side secret grants for an owner.
    CanManageSecretGrants,
    /// Accept an offered import at a target.
    CanAcceptSecretImport,
    /// Bind a brokered import at a target.
    CanBindBrokeredSecret,
    /// Bind a raw-delivery import at a target.
    CanBindRawSecret,
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
            Self::CanPublish => "can_publish",
            Self::CanRevoke => "can_revoke",
            Self::CanUse => "can_use",
            Self::CanUpdate => "can_update",
            Self::CanRecover => "can_recover",
            Self::InspectMetadata => "inspect_metadata",
            Self::WriteValue => "write_value",
            Self::Rotate => "rotate",
            Self::ManageGrants => "manage_grants",
            Self::Revoke => "revoke",
            Self::Purge => "purge",
            Self::Accept => "accept",
            Self::BindBrokered => "bind_brokered",
            Self::BindRaw => "bind_raw",
            Self::UseBrokered => "use_brokered",
            Self::ReceiveRaw => "receive_raw",
            Self::CanWriteSecretValue => "can_write_secret_value",
            Self::CanManageSecretGrants => "can_manage_secret_grants",
            Self::CanAcceptSecretImport => "can_accept_secret_import",
            Self::CanBindBrokeredSecret => "can_bind_brokered_secret",
            Self::CanBindRawSecret => "can_bind_raw_secret",
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
            "can_publish" => Ok(Self::CanPublish),
            "can_revoke" => Ok(Self::CanRevoke),
            "can_use" => Ok(Self::CanUse),
            "can_update" => Ok(Self::CanUpdate),
            "can_recover" => Ok(Self::CanRecover),
            "inspect_metadata" => Ok(Self::InspectMetadata),
            "write_value" => Ok(Self::WriteValue),
            "rotate" => Ok(Self::Rotate),
            "manage_grants" => Ok(Self::ManageGrants),
            "revoke" => Ok(Self::Revoke),
            "purge" => Ok(Self::Purge),
            "accept" => Ok(Self::Accept),
            "bind_brokered" => Ok(Self::BindBrokered),
            "bind_raw" => Ok(Self::BindRaw),
            "use_brokered" => Ok(Self::UseBrokered),
            "receive_raw" => Ok(Self::ReceiveRaw),
            "can_write_secret_value" => Ok(Self::CanWriteSecretValue),
            "can_manage_secret_grants" => Ok(Self::CanManageSecretGrants),
            "can_accept_secret_import" => Ok(Self::CanAcceptSecretImport),
            "can_bind_brokered_secret" => Ok(Self::CanBindBrokeredSecret),
            "can_bind_raw_secret" => Ok(Self::CanBindRawSecret),
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
    /// The configured authorization evaluator failed.
    #[error("authorization evaluator failed: {0}")]
    Evaluator(#[source] Box<dyn Error + Send + Sync>),
}

impl AuthzError {
    /// Wraps a provider-specific evaluator failure without exposing its type.
    #[must_use]
    pub fn evaluator(error: impl Error + Send + Sync + 'static) -> Self {
        Self::Evaluator(Box::new(error))
    }
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
        assert_eq!(
            Permission::from_str("can_revoke").expect("known permission"),
            Permission::CanRevoke
        );
        assert!(Permission::from_str("owner").is_err());
    }
}
