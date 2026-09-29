//! Canonical authorization subjects and resource references.

use identity_domain::UserId;
use runtime_types::{AgentInstanceId, RunId};
use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};
use uuid::Uuid;

use super::AuthzError;

/// An authorization subject.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Subject {
    /// One authenticated internal user.
    User(UserId),
    /// One authenticated exact runtime run.
    Run(RunId),
    /// One durable agent instance acting through an exact capability binding.
    AgentInstance(AgentInstanceId),
}

impl Subject {
    /// Returns the compiler subject type.
    #[must_use]
    pub const fn object_type(self) -> &'static str {
        match self {
            Self::User(_) => "user",
            Self::Run(_) => "run",
            Self::AgentInstance(_) => "agent_instance",
        }
    }

    /// Returns the stable textual subject identifier.
    #[must_use]
    pub fn id(self) -> String {
        match self {
            Self::User(id) => id.to_string(),
            Self::Run(id) => id.to_string(),
            Self::AgentInstance(id) => id.to_string(),
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
    /// Immutable OCI image produced from repository source.
    RepositoryOciImage,
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
    /// Project-owned HTTP gateway workload.
    Gateway,
    /// Immutable HTTP gateway declaration revision.
    GatewayRevision,
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
            Self::RepositoryOciImage => "repository_oci_image",
            Self::Run => "run",
            Self::StateVolume => "state_volume",
            Self::Build => "build",
            Self::Release => "release",
            Self::ReleaseAgent => "release_agent",
            Self::AgentInstance => "agent_instance",
            Self::Gateway => "gateway",
            Self::GatewayRevision => "gateway_revision",
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
            "repository_oci_image" => Ok(Self::RepositoryOciImage),
            "run" => Ok(Self::Run),
            "state_volume" => Ok(Self::StateVolume),
            "build" => Ok(Self::Build),
            "release" => Ok(Self::Release),
            "release_agent" => Ok(Self::ReleaseAgent),
            "agent_instance" => Ok(Self::AgentInstance),
            "gateway" => Ok(Self::Gateway),
            "gateway_revision" => Ok(Self::GatewayRevision),
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
