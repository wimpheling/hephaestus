//! Canonical authorization permissions.

use serde::{Deserialize, Serialize};
use std::str::FromStr;

use super::AuthzError;

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
    /// Grant an agent instance an explicit capability on a resource.
    CanGrantAgentCapability,
    /// Inspect a resource through an exact agent capability binding.
    AgentInspect,
    /// Configure a resource through an exact agent capability binding.
    AgentConfigure,
    /// Execute a resource operation through an exact agent capability binding.
    AgentExecute,
    /// Update a resource through an exact agent capability binding.
    AgentUpdate,
    /// Pause a resource through an exact agent capability binding.
    AgentPause,
    /// Recover a resource through an exact agent capability binding.
    AgentRecover,
    /// Read a repository through an exact agent capability binding.
    AgentGitRead,
    /// Create a repository ref through an exact agent capability binding.
    AgentCreateRef,
    /// Fast-forward a repository ref through an exact agent capability binding.
    AgentUpdateRef,
    /// Force-update a repository ref through an exact agent capability binding.
    AgentForceUpdateRef,
    /// Delete a repository ref through an exact agent capability binding.
    AgentDeleteRef,
    /// Create a repository tag through an exact agent capability binding.
    AgentCreateTag,
    /// Delete a repository tag through an exact agent capability binding.
    AgentDeleteTag,
    /// Trigger a run through an exact agent capability binding.
    AgentTriggerRun,
    /// Manage attachments through an exact agent capability binding.
    AgentManageAttachments,
    /// Cancel a run through an exact agent capability binding.
    AgentCancel,
    /// Attach a state volume through an exact agent capability binding.
    AgentAttach,
    /// Restore a state volume through an exact agent capability binding.
    AgentRestore,
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
            Self::CanGrantAgentCapability => "can_grant_agent_capability",
            Self::AgentInspect => "agent_inspect",
            Self::AgentConfigure => "agent_configure",
            Self::AgentExecute => "agent_execute",
            Self::AgentUpdate => "agent_update",
            Self::AgentPause => "agent_pause",
            Self::AgentRecover => "agent_recover",
            Self::AgentGitRead => "agent_git_read",
            Self::AgentCreateRef => "agent_create_ref",
            Self::AgentUpdateRef => "agent_update_ref",
            Self::AgentForceUpdateRef => "agent_force_update_ref",
            Self::AgentDeleteRef => "agent_delete_ref",
            Self::AgentCreateTag => "agent_create_tag",
            Self::AgentDeleteTag => "agent_delete_tag",
            Self::AgentTriggerRun => "agent_trigger_run",
            Self::AgentManageAttachments => "agent_manage_attachments",
            Self::AgentCancel => "agent_cancel",
            Self::AgentAttach => "agent_attach",
            Self::AgentRestore => "agent_restore",
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
            "can_grant_agent_capability" => Ok(Self::CanGrantAgentCapability),
            "agent_inspect" => Ok(Self::AgentInspect),
            "agent_configure" => Ok(Self::AgentConfigure),
            "agent_execute" => Ok(Self::AgentExecute),
            "agent_update" => Ok(Self::AgentUpdate),
            "agent_pause" => Ok(Self::AgentPause),
            "agent_recover" => Ok(Self::AgentRecover),
            "agent_git_read" => Ok(Self::AgentGitRead),
            "agent_create_ref" => Ok(Self::AgentCreateRef),
            "agent_update_ref" => Ok(Self::AgentUpdateRef),
            "agent_force_update_ref" => Ok(Self::AgentForceUpdateRef),
            "agent_delete_ref" => Ok(Self::AgentDeleteRef),
            "agent_create_tag" => Ok(Self::AgentCreateTag),
            "agent_delete_tag" => Ok(Self::AgentDeleteTag),
            "agent_trigger_run" => Ok(Self::AgentTriggerRun),
            "agent_manage_attachments" => Ok(Self::AgentManageAttachments),
            "agent_cancel" => Ok(Self::AgentCancel),
            "agent_attach" => Ok(Self::AgentAttach),
            "agent_restore" => Ok(Self::AgentRestore),
            _ => Err(AuthzError::UnknownPermission(value.to_owned())),
        }
    }
}
