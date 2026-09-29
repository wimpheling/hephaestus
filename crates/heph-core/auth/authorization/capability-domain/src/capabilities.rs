use serde::{Deserialize, Serialize};
use std::fmt;

use crate::{CapabilityError, MAX_CAPABILITY_SLOT_KEY_BYTES};

/// A stable symbolic capability name declared by released code.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct CapabilitySlotKey(String);

impl CapabilitySlotKey {
    /// Parses a bounded lowercase key.
    ///
    /// # Errors
    ///
    /// Returns [`CapabilityError::InvalidSlotKey`] unless the value starts
    /// with a lowercase ASCII letter and contains only lowercase letters,
    /// digits, underscores, or hyphens.
    pub fn parse(value: impl Into<String>) -> Result<Self, CapabilityError> {
        let value = value.into();
        let mut bytes = value.bytes();
        let valid = !value.is_empty()
            && value.len() <= MAX_CAPABILITY_SLOT_KEY_BYTES
            && bytes.next().is_some_and(|byte| byte.is_ascii_lowercase())
            && bytes.all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_' || byte == b'-'
            });
        if !valid {
            return Err(CapabilityError::InvalidSlotKey);
        }
        Ok(Self(value))
    }

    /// Returns the validated key.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for CapabilitySlotKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl TryFrom<String> for CapabilitySlotKey {
    type Error = CapabilityError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<CapabilitySlotKey> for String {
    fn from(value: CapabilitySlotKey) -> Self {
        value.0
    }
}

/// Resource categories which may be selected for a capability slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityResourceKind {
    /// A project-owned Git repository.
    Repository,
    /// A project boundary.
    Project,
    /// A durable agent instance.
    AgentInstance,
    /// A declared HTTP gateway.
    Gateway,
    /// One exact execution.
    Run,
    /// A persistent private state volume.
    StateVolume,
    /// A durable agent-instance mailbox.
    Mailbox,
}

impl CapabilityResourceKind {
    /// Returns the stable wire name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Repository => "repository",
            Self::Project => "project",
            Self::AgentInstance => "agent_instance",
            Self::Gateway => "gateway",
            Self::Run => "run",
            Self::StateVolume => "state_volume",
            Self::Mailbox => "mailbox",
        }
    }
}

/// Closed semantic operation vocabulary for capability ceilings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityOperation {
    /// Inspect resource metadata.
    Inspect,
    /// Change resource configuration.
    Configure,
    /// Execute or invoke the resource.
    Execute,
    /// Apply an update to the resource.
    Update,
    /// Pause new work.
    Pause,
    /// Recover paused or failed work.
    Recover,
    /// Cancel an exact execution.
    Cancel,
    /// Attach a resource to a workload.
    Attach,
    /// Restore an earlier durable state.
    Restore,
    /// Fetch raw Git refs and objects.
    GitRead,
    /// Create a Git ref.
    CreateRef,
    /// Fast-forward an existing Git ref.
    UpdateRef,
    /// Non-fast-forward update of an existing Git ref.
    ForceUpdateRef,
    /// Delete a Git ref.
    DeleteRef,
    /// Create a Git tag.
    CreateTag,
    /// Delete a Git tag.
    DeleteTag,
    /// Trigger execution from an accepted repository update.
    TriggerRun,
    /// Manage repository/ref attachments.
    ManageAttachments,
    /// Publish one bounded generic event to a mailbox.
    Publish,
}

impl CapabilityOperation {
    /// Returns the stable wire name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Inspect => "inspect",
            Self::Configure => "configure",
            Self::Execute => "execute",
            Self::Update => "update",
            Self::Pause => "pause",
            Self::Recover => "recover",
            Self::Cancel => "cancel",
            Self::Attach => "attach",
            Self::Restore => "restore",
            Self::GitRead => "git_read",
            Self::CreateRef => "create_ref",
            Self::UpdateRef => "update_ref",
            Self::ForceUpdateRef => "force_update_ref",
            Self::DeleteRef => "delete_ref",
            Self::CreateTag => "create_tag",
            Self::DeleteTag => "delete_tag",
            Self::TriggerRun => "trigger_run",
            Self::ManageAttachments => "manage_attachments",
            Self::Publish => "publish",
        }
    }

    /// Returns whether the operation is defined for the resource category.
    #[must_use]
    pub const fn is_legal_for(self, resource_kind: CapabilityResourceKind) -> bool {
        use CapabilityOperation::{
            Attach, Cancel, Configure, CreateRef, CreateTag, DeleteRef, DeleteTag, Execute,
            ForceUpdateRef, GitRead, Inspect, ManageAttachments, Pause, Publish, Recover, Restore,
            TriggerRun, Update, UpdateRef,
        };
        use CapabilityResourceKind::{
            AgentInstance, Gateway, Mailbox, Project, Repository, Run, StateVolume,
        };

        match resource_kind {
            Repository => matches!(
                self,
                Inspect
                    | GitRead
                    | CreateRef
                    | UpdateRef
                    | ForceUpdateRef
                    | DeleteRef
                    | CreateTag
                    | DeleteTag
                    | TriggerRun
                    | ManageAttachments
            ),
            Project | AgentInstance | Gateway => {
                matches!(
                    self,
                    Inspect | Configure | Execute | Update | Pause | Recover
                )
            }
            Run => matches!(self, Inspect | Cancel | Recover),
            StateVolume => matches!(self, Inspect | Attach | Restore),
            Mailbox => matches!(self, Publish),
        }
    }
}
