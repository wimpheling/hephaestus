//! Typed internal command vocabulary.

use capability_domain::{CapabilityOperation, CapabilityResource, CapabilitySlotKey};
use forge_domain::{ProjectId, RepositoryId};
use release_domain::{
    AgentAttachmentId, AgentInstanceId, AgentInstanceRevisionId, AgentUpdateId, InstanceName,
    ParameterName, ParameterValue, RefSelector, ReleaseAgentId, RuntimePolicy, TriggerPolicy,
};
use release_postgres::BrokeredRuleCopy;
use secret_domain::{
    AgentSecretBindingId, DeliveryMode, ExecutionPhase, SecretAlias, SecretGrantId, SecretId,
    SecretImportId, SecretName, SecretOwner, SecretSlotKey, SecretTarget, SecretUsePolicy,
    SecretVersionId,
};
use std::collections::BTreeMap;
use time::OffsetDateTime;
use uuid::Uuid;

pub enum InternalCommand {
    ImportAgent {
        project_id: ProjectId,
        release_agent_id: ReleaseAgentId,
        name: InstanceName,
        parameters: BTreeMap<ParameterName, ParameterValue>,
        selected_policy: RuntimePolicy,
    },
    CreateAttachment {
        instance_id: AgentInstanceId,
        repository_id: RepositoryId,
        ref_selector: RefSelector,
        trigger_policy: TriggerPolicy,
    },
    SetAttachmentEnabled {
        attachment_id: AgentAttachmentId,
        enabled: bool,
    },
    RemoveAttachment {
        attachment_id: AgentAttachmentId,
    },
    ReviseInstance {
        instance_id: AgentInstanceId,
        expected_revision_id: AgentInstanceRevisionId,
        parameters: BTreeMap<ParameterName, ParameterValue>,
        selected_policy: RuntimePolicy,
    },
    ReviseCapabilities {
        instance_id: AgentInstanceId,
        expected_revision_id: AgentInstanceRevisionId,
        bindings: Vec<CapabilitySelectionInput>,
    },
    CreateUpdate {
        instance_id: AgentInstanceId,
        expected_revision_id: AgentInstanceRevisionId,
        candidate_release_agent_id: ReleaseAgentId,
        parameters: BTreeMap<ParameterName, ParameterValue>,
        brokered_rule_copies: Vec<BrokeredRuleCopy>,
        selected_policy: RuntimePolicy,
    },
    RecoverUpdate {
        update_id: AgentUpdateId,
        action: RecoveryAction,
    },
    CreateSecret {
        owner: SecretOwner,
        name: SecretName,
        allowed_delivery_modes: Vec<DeliveryMode>,
        value: Vec<u8>,
    },
    RotateSecret {
        secret_id: SecretId,
        expected_active_version_id: SecretVersionId,
        value: Vec<u8>,
    },
    GrantSecret {
        secret_id: SecretId,
        target: SecretTarget,
        policy: SecretUsePolicy,
        expires_at: Option<OffsetDateTime>,
    },
    AcceptSecretImport {
        grant_id: SecretGrantId,
        target: SecretTarget,
        alias: SecretAlias,
    },
    BindSecret {
        instance_id: AgentInstanceId,
        expected_revision_id: AgentInstanceRevisionId,
        import_id: SecretImportId,
        slot: SecretSlotKey,
        mode: DeliveryMode,
        phases: Vec<ExecutionPhase>,
        attachment_ids: Vec<Uuid>,
        destinations: Vec<String>,
    },
    DeclareBrokeredHttpsRule {
        binding_id: AgentSecretBindingId,
        destination: String,
        header: String,
        header_prefix: Option<String>,
        requested_rule_id: Option<Uuid>,
    },
    SetSecretEnabled {
        secret_id: SecretId,
        enabled: bool,
    },
    RevokeSecret {
        secret_id: SecretId,
    },
    PurgeSecret {
        secret_id: SecretId,
    },
}

pub struct CapabilitySelectionInput {
    pub slot: CapabilitySlotKey,
    pub resource: CapabilityResource,
    pub granted_operations: Vec<CapabilityOperation>,
}

#[derive(Clone, Copy)]
pub enum RecoveryAction {
    Retry,
    Reject,
    Resume,
}
