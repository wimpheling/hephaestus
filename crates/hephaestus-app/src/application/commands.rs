//! Shared typed command application operations.
use capability_domain::{
    CapabilityBindingId, CapabilityOperation, CapabilityResource, CapabilitySlotKey,
};
use forge_domain::{ProjectId, RepositoryId};
use identity_domain::AuthenticatedIdentity;
use release_domain::{
    AgentAttachmentId, AgentInstanceId, AgentInstanceRevisionId, AgentUpdateId, InstanceName,
    ParameterName, ParameterValue, RefSelector, ReleaseAgentId, ReleaseCommandKey, RuntimePolicy,
    TriggerPolicy,
};
use release_postgres::{
    BeginUpdateHook, BrokeredRuleCopy, CapabilityBindingSelection, CreateAttachment,
    CreateInstanceUpdate, ImportAgent, RecoverInstanceUpdate, ReleaseService, RemoveAttachment,
    ReviseInstance, ReviseInstanceCapabilities, SetAttachmentEnabled, UpdateRecoveryAction,
};
use runtime_types::RunId;
use secret_application::{
    AcceptSecretImport, BindSecret, CreateSecret, DeclareBrokeredHttpsRule, GrantSecret,
    RotateSecret,
};
use secret_domain::{
    AgentSecretBindingId, DeliveryMode, ExecutionPhase, SecretAlias, SecretCommandKey,
    SecretGrantId, SecretId, SecretImportId, SecretName, SecretOwner, SecretSlotKey, SecretTarget,
    SecretUsePolicy, SecretValue, SecretVersionId,
};
use secret_postgres::SecretService;
use secret_store::LocalKeyProvider;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
#[cfg(feature = "test-fixtures")]
use std::sync::{Mutex, OnceLock};
use std::{collections::BTreeMap, sync::Arc};
use time::OffsetDateTime;
#[cfg(feature = "test-fixtures")]
use tokio::sync::Notify;
use uuid::Uuid;

/// Test-only synchronization point for the durable update admission race.
///
/// The hook is inert unless an integration test explicitly installs it. It
/// pauses after `CreateUpdate` commits and before the immediate hook attempt,
/// allowing the durable completion reconciler to win that admission race.
#[doc(hidden)]
#[cfg(feature = "test-fixtures")]
pub struct CreateUpdateAdmissionBarrier {
    committed: Notify,
    admitted: Notify,
    release: Notify,
    committed_update: Mutex<Option<Uuid>>,
    admitted_update: Mutex<Option<Uuid>>,
}

#[cfg(feature = "test-fixtures")]
impl CreateUpdateAdmissionBarrier {
    /// Creates an untriggered admission barrier.
    #[must_use]
    pub fn new() -> Self {
        Self {
            committed: Notify::new(),
            admitted: Notify::new(),
            release: Notify::new(),
            committed_update: Mutex::new(None),
            admitted_update: Mutex::new(None),
        }
    }

    /// Waits until the update transaction has committed.
    ///
    /// # Panics
    ///
    /// Panics if the test barrier mutex is poisoned.
    pub async fn wait_committed(&self) -> Uuid {
        loop {
            let notification = self.committed.notified();
            let committed_update = *self
                .committed_update
                .lock()
                .expect("committed update mutex");
            if let Some(update_id) = committed_update {
                return update_id;
            }
            notification.await;
        }
    }

    /// Waits until the durable reconciler admits the hook run.
    ///
    /// # Panics
    ///
    /// Panics if the test barrier mutex is poisoned.
    pub async fn wait_admitted(&self, update_id: Uuid) {
        loop {
            let notification = self.admitted.notified();
            let admitted_update = *self.admitted_update.lock().expect("admitted update mutex");
            if admitted_update == Some(update_id) {
                return;
            }
            notification.await;
        }
    }

    /// Releases the immediate application admission attempt.
    pub fn release(&self) {
        self.release.notify_one();
    }

    async fn pause_before_immediate_attempt(&self, update_id: Uuid) {
        *self
            .committed_update
            .lock()
            .expect("committed update mutex") = Some(update_id);
        self.committed.notify_one();
        self.release.notified().await;
    }

    pub(crate) fn notify_reconciler_admission(&self, update_id: Uuid) {
        if *self
            .committed_update
            .lock()
            .expect("committed update mutex")
            == Some(update_id)
        {
            *self.admitted_update.lock().expect("admitted update mutex") = Some(update_id);
            self.admitted.notify_one();
        }
    }
}

#[cfg(feature = "test-fixtures")]
impl Default for CreateUpdateAdmissionBarrier {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "test-fixtures")]
static CREATE_UPDATE_ADMISSION_BARRIER: OnceLock<Mutex<Option<Arc<CreateUpdateAdmissionBarrier>>>> =
    OnceLock::new();

/// Installs the integration-test admission barrier until the returned guard
/// is dropped. Only one barrier may be active in a process.
#[doc(hidden)]
#[cfg(feature = "test-fixtures")]
pub fn install_create_update_admission_barrier(
    barrier: Arc<CreateUpdateAdmissionBarrier>,
) -> CreateUpdateAdmissionBarrierGuard {
    let slot = CREATE_UPDATE_ADMISSION_BARRIER.get_or_init(|| Mutex::new(None));
    let mut current = slot.lock().expect("update admission barrier mutex");
    assert!(
        current.is_none(),
        "an update admission barrier is already active"
    );
    *current = Some(barrier);
    CreateUpdateAdmissionBarrierGuard
}

/// Removes an installed integration-test admission barrier on scope exit.
#[doc(hidden)]
#[cfg(feature = "test-fixtures")]
pub struct CreateUpdateAdmissionBarrierGuard;

#[cfg(feature = "test-fixtures")]
impl Drop for CreateUpdateAdmissionBarrierGuard {
    fn drop(&mut self) {
        if let Some(slot) = CREATE_UPDATE_ADMISSION_BARRIER.get() {
            *slot.lock().expect("update admission barrier mutex") = None;
        }
    }
}

#[cfg(feature = "test-fixtures")]
fn installed_create_update_admission_barrier() -> Option<Arc<CreateUpdateAdmissionBarrier>> {
    CREATE_UPDATE_ADMISSION_BARRIER
        .get()
        .and_then(|slot| slot.lock().ok()?.clone())
}

/// Signals an installed test barrier when durable reconciliation admits a
/// pending update hook.
#[doc(hidden)]
#[cfg(feature = "test-fixtures")]
pub fn notify_reconciler_update_admission(update_id: Uuid) {
    if let Some(barrier) = installed_create_update_admission_barrier() {
        barrier.notify_reconciler_admission(update_id);
    }
}

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

struct CreateUpdateDispatch {
    instance_id: AgentInstanceId,
    expected_revision_id: AgentInstanceRevisionId,
    candidate_release_agent_id: ReleaseAgentId,
    parameters: BTreeMap<ParameterName, ParameterValue>,
    brokered_rule_copies: Vec<BrokeredRuleCopy>,
    selected_policy: RuntimePolicy,
}

async fn dispatch_create_update(
    state: &InternalCommandState,
    identity: &AuthenticatedIdentity,
    input: CreateUpdateDispatch,
) -> Result<Value, Box<dyn std::error::Error>> {
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
                instance_id: input.instance_id,
                expected_revision_id: input.expected_revision_id,
                candidate_revision_id: revision_id,
                candidate_release_agent_id: input.candidate_release_agent_id,
                parameters: input.parameters,
                brokered_rule_copies: input.brokered_rule_copies,
                selected_policy: input.selected_policy,
                platform_policy: state.platform_policy.clone(),
                platform_policy_version: state.platform_policy_version.clone(),
            },
        )
        .await?;
    #[cfg(feature = "test-fixtures")]
    if let Some(barrier) = installed_create_update_admission_barrier() {
        barrier
            .pause_before_immediate_attempt(update_id.as_uuid())
            .await;
    }
    let hook_run_id = RunId::from_uuid(stable_id(identity, "create_update.hook_run"));
    let admitted_hook_run_id = match state
        .releases
        .begin_update_hook(
            identity,
            BeginUpdateHook {
                command_key: release_key(identity, "begin_update_hook", hook_run_id.as_uuid()),
                update_id,
                hook_run_id,
            },
        )
        .await
    {
        Ok(()) => Some(hook_run_id),
        // create_update has already durably closed the gate. Keep the
        // accepted update fenced until the completion observer can
        // re-authorize and admit its hook after normal cleanup.
        Err(release_postgres::ReleaseServiceError::UpdateDrainPending) => None,
        Err(error)
            if matches!(
                &error,
                release_postgres::ReleaseServiceError::InvalidUpdateLifecycle
            ) =>
        {
            let admitted = state
                .releases
                .current_update_hook_run(identity, update_id)
                .await?;
            if let Some(admitted) = admitted {
                Some(admitted)
            } else {
                return Err(error.into());
            }
        }
        Err(error) => return Err(error.into()),
    };
    Ok(json!({
        "update_id": update_id,
        "candidate_revision_id": revision_id,
        "hook_run_id": admitted_hook_run_id,
    }))
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
        InternalCommand::ReviseCapabilities {
            instance_id,
            expected_revision_id,
            bindings,
        } => {
            let revision_id = AgentInstanceRevisionId::from_uuid(stable_id(
                identity,
                "revise_instance_capabilities.revision",
            ));
            let bindings = bindings
                .into_iter()
                .map(|binding| CapabilityBindingSelection {
                    binding_id: CapabilityBindingId::from_uuid(stable_id(
                        identity,
                        &format!("revise_instance_capabilities.binding.{}", binding.slot),
                    )),
                    slot: binding.slot,
                    resource: binding.resource,
                    granted_operations: binding.granted_operations,
                    // Typed runtime Git scope is delivered by the MVP 01.3
                    // transport; this generic capability form cannot invent it.
                    git_authority: None,
                })
                .collect();
            let result = state
                .releases
                .revise_instance_capabilities(
                    identity,
                    ReviseInstanceCapabilities {
                        command_key: release_key(
                            identity,
                            "revise_instance_capabilities",
                            instance_id.as_uuid(),
                        ),
                        instance_id,
                        expected_revision_id,
                        new_revision_id: revision_id,
                        bindings,
                        authorization_model_version: String::from(
                            authz_postgres::AUTHORIZATION_MODEL_VERSION,
                        ),
                    },
                )
                .await?;
            Ok(json!({
                "instance_id": instance_id,
                "revision_id": result.revision_id,
                "runnable": result.runnable,
                "diagnostics": result.diagnostics.into_iter().map(|diagnostic| json!({
                    "code": diagnostic.code,
                    "slot": diagnostic.slot,
                })).collect::<Vec<_>>(),
            }))
        }
        InternalCommand::CreateUpdate {
            instance_id,
            expected_revision_id,
            candidate_release_agent_id,
            parameters,
            brokered_rule_copies,
            selected_policy,
        } => {
            dispatch_create_update(
                state,
                identity,
                CreateUpdateDispatch {
                    instance_id,
                    expected_revision_id,
                    candidate_release_agent_id,
                    parameters,
                    brokered_rule_copies,
                    selected_policy,
                },
            )
            .await
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
        InternalCommand::DeclareBrokeredHttpsRule {
            binding_id,
            destination,
            header,
            header_prefix,
        } => {
            let rule_id = stable_id(identity, "declare_brokered_https_rule.rule");
            state
                .secrets
                .declare_brokered_https_rule(
                    identity,
                    DeclareBrokeredHttpsRule {
                        command_key: secret_key(identity, "declare_brokered_https_rule", rule_id),
                        rule_id,
                        binding_id,
                        destination,
                        header,
                        header_prefix,
                    },
                )
                .await?;
            Ok(json!({ "rule_id": rule_id }))
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
