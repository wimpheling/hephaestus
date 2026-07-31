//! Shared typed command application operations.
use forge_domain::{ProjectId, RepositoryId};
use identity_domain::AuthenticatedIdentity;
use release_domain::{
    AgentAttachmentId, AgentInstanceId, AgentInstanceRevisionId, AgentUpdateId, InstanceName,
    ParameterName, ParameterValue, RefSelector, ReleaseAgentId, ReleaseCommandKey, RuntimePolicy,
    TriggerPolicy,
};
use release_postgres::{
    BeginUpdateHook, CreateAttachment, CreateInstanceUpdate, ImportAgent, RecoverInstanceUpdate,
    ReleaseService, RemoveAttachment, ReviseInstance, SetAttachmentEnabled, UpdateRecoveryAction,
};
use runtime_types::RunId;
use secret_application::{AcceptSecretImport, BindSecret, CreateSecret, GrantSecret, RotateSecret};
use secret_domain::{
    AgentSecretBindingId, DeliveryMode, ExecutionPhase, SecretAlias, SecretCommandKey,
    SecretGrantId, SecretId, SecretImportId, SecretName, SecretOwner, SecretSlotKey, SecretTarget,
    SecretUsePolicy, SecretValue, SecretVersionId,
};
use secret_postgres::SecretService;
use secret_store::LocalKeyProvider;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, sync::Arc};
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Clone)]
pub struct InternalCommandState {
    releases: Arc<ReleaseService>,
    secrets: Arc<SecretService<LocalKeyProvider>>,
    platform_policy: RuntimePolicy,
    platform_policy_version: String,
}

impl InternalCommandState {
    pub(crate) const fn new(
        releases: Arc<ReleaseService>,
        secrets: Arc<SecretService<LocalKeyProvider>>,
        platform_policy: RuntimePolicy,
        platform_policy_version: String,
    ) -> Self {
        Self {
            releases,
            secrets,
            platform_policy,
            platform_policy_version,
        }
    }
}

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
    CreateUpdate {
        instance_id: AgentInstanceId,
        expected_revision_id: AgentInstanceRevisionId,
        candidate_release_agent_id: ReleaseAgentId,
        parameters: BTreeMap<ParameterName, ParameterValue>,
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

#[derive(Clone, Copy)]
pub enum RecoveryAction {
    Retry,
    Reject,
    Resume,
}

// Keeping the command-to-domain mapping together makes the trusted internal
// boundary and its complete browser payload vocabulary directly auditable.
#[allow(clippy::too_many_lines)]
pub async fn dispatch(
    state: &InternalCommandState,
    identity: &AuthenticatedIdentity,
    command: InternalCommand,
) -> Result<Value, Box<dyn std::error::Error>> {
    match command {
        InternalCommand::ImportAgent {
            project_id,
            release_agent_id,
            name,
            parameters,
            selected_policy,
        } => {
            let instance_id =
                AgentInstanceId::from_uuid(stable_id(identity, "import_agent.instance"));
            let revision_id =
                AgentInstanceRevisionId::from_uuid(stable_id(identity, "import_agent.revision"));
            state
                .releases
                .import_agent(
                    identity,
                    ImportAgent {
                        command_key: release_key(identity, "import_agent", instance_id.as_uuid()),
                        instance_id,
                        revision_id,
                        project_id,
                        release_agent_id,
                        name,
                        parameters,
                        selected_policy,
                        platform_policy: state.platform_policy.clone(),
                        platform_policy_version: state.platform_policy_version.clone(),
                    },
                )
                .await?;
            Ok(json!({"instance_id": instance_id, "revision_id": revision_id}))
        }
        InternalCommand::CreateAttachment {
            instance_id,
            repository_id,
            ref_selector,
            trigger_policy,
        } => {
            let attachment_id =
                AgentAttachmentId::from_uuid(stable_id(identity, "create_attachment"));
            state
                .releases
                .create_attachment(
                    identity,
                    CreateAttachment {
                        command_key: release_key(
                            identity,
                            "create_attachment",
                            attachment_id.as_uuid(),
                        ),
                        attachment_id,
                        instance_id,
                        repository_id,
                        ref_selector,
                        trigger_policy,
                    },
                )
                .await?;
            Ok(json!({"attachment_id": attachment_id}))
        }
        InternalCommand::SetAttachmentEnabled {
            attachment_id,
            enabled,
        } => {
            state
                .releases
                .set_attachment_enabled(
                    identity,
                    SetAttachmentEnabled {
                        command_key: release_key(
                            identity,
                            "set_attachment_enabled",
                            attachment_id.as_uuid(),
                        ),
                        attachment_id,
                        enabled,
                    },
                )
                .await?;
            Ok(json!({"attachment_id": attachment_id, "enabled": enabled}))
        }
        InternalCommand::RemoveAttachment { attachment_id } => {
            state
                .releases
                .remove_attachment(
                    identity,
                    RemoveAttachment {
                        command_key: release_key(
                            identity,
                            "remove_attachment",
                            attachment_id.as_uuid(),
                        ),
                        attachment_id,
                    },
                )
                .await?;
            Ok(json!({"attachment_id": attachment_id, "state": "removed"}))
        }
        InternalCommand::ReviseInstance {
            instance_id,
            expected_revision_id,
            parameters,
            selected_policy,
        } => {
            let revision_id =
                AgentInstanceRevisionId::from_uuid(stable_id(identity, "revise_instance"));
            state
                .releases
                .revise_instance(
                    identity,
                    ReviseInstance {
                        command_key: release_key(
                            identity,
                            "revise_instance",
                            revision_id.as_uuid(),
                        ),
                        instance_id,
                        expected_revision_id,
                        new_revision_id: revision_id,
                        parameters,
                        selected_policy,
                        platform_policy: state.platform_policy.clone(),
                        platform_policy_version: state.platform_policy_version.clone(),
                    },
                )
                .await?;
            Ok(json!({"instance_id": instance_id, "revision_id": revision_id}))
        }
        InternalCommand::CreateUpdate {
            instance_id,
            expected_revision_id,
            candidate_release_agent_id,
            parameters,
            selected_policy,
        } => {
            let update_id = AgentUpdateId::from_uuid(stable_id(identity, "create_update.update"));
            let revision_id =
                AgentInstanceRevisionId::from_uuid(stable_id(identity, "create_update.revision"));
            state
                .releases
                .create_update(
                    identity,
                    CreateInstanceUpdate {
                        command_key: release_key(identity, "create_update", update_id.as_uuid()),
                        update_id,
                        instance_id,
                        expected_revision_id,
                        candidate_revision_id: revision_id,
                        candidate_release_agent_id,
                        parameters,
                        selected_policy,
                        platform_policy: state.platform_policy.clone(),
                        platform_policy_version: state.platform_policy_version.clone(),
                    },
                )
                .await?;
            let hook_run_id = RunId::from_uuid(stable_id(identity, "create_update.hook_run"));
            state
                .releases
                .begin_update_hook(
                    identity,
                    BeginUpdateHook {
                        command_key: release_key(
                            identity,
                            "begin_update_hook",
                            hook_run_id.as_uuid(),
                        ),
                        update_id,
                        hook_run_id,
                    },
                )
                .await?;
            Ok(json!({
                "update_id": update_id,
                "candidate_revision_id": revision_id,
                "hook_run_id": hook_run_id,
            }))
        }
        InternalCommand::RecoverUpdate { update_id, action } => {
            let action = match action {
                RecoveryAction::Retry => UpdateRecoveryAction::RetryHook,
                RecoveryAction::Reject => UpdateRecoveryAction::RejectCandidate,
                RecoveryAction::Resume => UpdateRecoveryAction::ResumeActivation,
            };
            let decision = state
                .releases
                .recover_update(
                    identity,
                    RecoverInstanceUpdate {
                        command_key: release_key(identity, "recover_update", update_id.as_uuid()),
                        update_id,
                        action,
                    },
                )
                .await?;
            Ok(json!({"update_id": update_id, "decision": format!("{decision:?}")}))
        }
        InternalCommand::CreateSecret {
            owner,
            name,
            allowed_delivery_modes,
            value,
        } => {
            let secret_id = SecretId::from_uuid(stable_id(identity, "create_secret.secret"));
            let version_id =
                SecretVersionId::from_uuid(stable_id(identity, "create_secret.version"));
            let created = state
                .secrets
                .create(
                    identity,
                    CreateSecret {
                        command_key: secret_key(identity, "create_secret", secret_id.as_uuid()),
                        secret_id,
                        version_id,
                        owner,
                        name,
                        allowed_delivery_modes,
                        value: SecretValue::new(value)?,
                    },
                )
                .await?;
            Ok(json!({
                "secret_id": created.secret_id,
                "secret_version_id": created.version_id,
            }))
        }
        InternalCommand::RotateSecret {
            secret_id,
            expected_active_version_id,
            value,
        } => {
            let version_id = SecretVersionId::from_uuid(stable_id(identity, "rotate_secret"));
            state
                .secrets
                .rotate(
                    identity,
                    RotateSecret {
                        command_key: secret_key(identity, "rotate_secret", version_id.as_uuid()),
                        secret_id,
                        expected_active_version_id,
                        new_version_id: version_id,
                        value: SecretValue::new(value)?,
                    },
                )
                .await?;
            Ok(json!({"secret_id": secret_id, "secret_version_id": version_id}))
        }
        InternalCommand::GrantSecret {
            secret_id,
            target,
            policy,
            expires_at,
        } => {
            let grant_id = SecretGrantId::from_uuid(stable_id(identity, "grant_secret"));
            state
                .secrets
                .grant(
                    identity,
                    GrantSecret {
                        command_key: secret_key(identity, "grant_secret", grant_id.as_uuid()),
                        grant_id,
                        secret_id,
                        target,
                        policy,
                        expires_at,
                    },
                )
                .await?;
            Ok(json!({"grant_id": grant_id}))
        }
        InternalCommand::AcceptSecretImport {
            grant_id,
            target,
            alias,
        } => {
            let import_id = SecretImportId::from_uuid(stable_id(identity, "accept_secret_import"));
            state
                .secrets
                .accept_import(
                    identity,
                    AcceptSecretImport {
                        command_key: secret_key(
                            identity,
                            "accept_secret_import",
                            import_id.as_uuid(),
                        ),
                        import_id,
                        grant_id,
                        target,
                        alias,
                    },
                )
                .await?;
            Ok(json!({"import_id": import_id}))
        }
        InternalCommand::BindSecret {
            instance_id,
            expected_revision_id,
            import_id,
            slot,
            mode,
            phases,
            attachment_ids,
            destinations,
        } => {
            let binding_id =
                AgentSecretBindingId::from_uuid(stable_id(identity, "bind_secret.binding"));
            let revision_id =
                AgentInstanceRevisionId::from_uuid(stable_id(identity, "bind_secret.revision"));
            state
                .secrets
                .bind_secret(
                    identity,
                    BindSecret {
                        command_key: secret_key(identity, "bind_secret", binding_id.as_uuid()),
                        binding_id,
                        instance_id,
                        expected_revision_id,
                        new_revision_id: revision_id,
                        import_id,
                        slot,
                        mode,
                        phases,
                        attachment_ids,
                        destinations,
                    },
                )
                .await?;
            Ok(json!({
                "binding_id": binding_id,
                "instance_revision_id": revision_id,
            }))
        }
        InternalCommand::SetSecretEnabled { secret_id, enabled } => {
            state
                .secrets
                .set_secret_enabled(
                    identity,
                    secret_key(identity, "set_secret_enabled", secret_id.as_uuid()),
                    secret_id,
                    enabled,
                )
                .await?;
            Ok(json!({
                "secret_id": secret_id,
                "status": if enabled { "active" } else { "disabled" },
            }))
        }
        InternalCommand::RevokeSecret { secret_id } => {
            state
                .secrets
                .revoke_secret(
                    identity,
                    secret_key(identity, "revoke_secret", secret_id.as_uuid()),
                    secret_id,
                )
                .await?;
            Ok(json!({"secret_id": secret_id, "state": "revoked"}))
        }
        InternalCommand::PurgeSecret { secret_id } => {
            state
                .secrets
                .purge_secret(
                    identity,
                    secret_key(identity, "purge_secret", secret_id.as_uuid()),
                    secret_id,
                )
                .await?;
            Ok(json!({"secret_id": secret_id, "state": "purged"}))
        }
    }
}

fn release_key(
    identity: &AuthenticatedIdentity,
    operation: &str,
    aggregate_id: Uuid,
) -> ReleaseCommandKey {
    ReleaseCommandKey::derive(
        operation,
        &[
            identity.idempotency_id.as_uuid().as_bytes(),
            aggregate_id.as_bytes(),
        ],
    )
}

fn secret_key(
    identity: &AuthenticatedIdentity,
    operation: &str,
    aggregate_id: Uuid,
) -> SecretCommandKey {
    SecretCommandKey::derive(
        operation,
        &[
            identity.idempotency_id.as_uuid().as_bytes(),
            aggregate_id.as_bytes(),
        ],
    )
}

fn stable_id(identity: &AuthenticatedIdentity, purpose: &str) -> Uuid {
    let mut digest = Sha256::new();
    digest.update(b"hephaestus-command-resource-v1\0");
    digest.update(identity.user_id.as_uuid().as_bytes());
    digest.update(identity.idempotency_id.as_uuid().as_bytes());
    digest.update(purpose.as_bytes());
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest.finalize()[..16]);
    // RFC 9562 version 8 is reserved for application-defined UUID layouts.
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes)
}

#[cfg(test)]
mod tests {
    use super::stable_id;
    use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
    use serde_json::json;

    #[test]
    fn retry_resource_ids_are_stable_and_operation_scoped() {
        let identity = AuthenticatedIdentity::new(
            UserId::new(),
            "test",
            "subject",
            json!({}),
            RequestId::new(),
        );
        let first = stable_id(&identity, "create_secret.secret");
        assert_eq!(first, stable_id(&identity, "create_secret.secret"));
        assert_ne!(first, stable_id(&identity, "create_secret.version"));
        assert_eq!(first.get_version_num(), 8);
    }
}
