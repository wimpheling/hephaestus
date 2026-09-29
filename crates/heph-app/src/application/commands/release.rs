//! Release and instance command execution.

#[cfg(feature = "test-fixtures")]
use super::barrier::installed_create_update_admission_barrier;
use super::{
    ids::{release_key, stable_id},
    state::InternalCommandState,
    types::{InternalCommand, RecoveryAction},
};
use capability_domain::CapabilityBindingId;
use identity_domain::AuthenticatedIdentity;
use release_domain::{
    AgentAttachmentId, AgentInstanceId, AgentInstanceRevisionId, AgentUpdateId, ParameterName,
    ParameterValue, ReleaseAgentId, RuntimePolicy,
};
use release_postgres::{
    BeginUpdateHook, BrokeredRuleCopy, CapabilityBindingSelection, CreateAttachment,
    CreateInstanceUpdate, ImportAgent, RecoverInstanceUpdate, RemoveAttachment, ReviseInstance,
    ReviseInstanceCapabilities, SetAttachmentEnabled, UpdateRecoveryAction,
};
use runtime_types::RunId;
use serde_json::{Value, json};
use std::{collections::BTreeMap, error::Error};

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

// Keep release command mapping together at the trusted application boundary.
#[allow(clippy::too_many_lines)]
pub(super) async fn dispatch_release(
    state: &InternalCommandState,
    identity: &AuthenticatedIdentity,
    command: InternalCommand,
) -> Result<Value, Box<dyn Error>> {
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

        _ => unreachable!("non-release command routed to release dispatcher"),
    }
}
