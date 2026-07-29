//! Trusted web-mediator command boundary.

use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode, header::AUTHORIZATION},
    routing::post,
};
use forge_domain::{ProjectId, RepositoryId};
use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
use release_domain::{
    AgentAttachmentId, AgentInstanceId, AgentInstanceRevisionId, AgentUpdateId, InstanceName,
    ParameterName, ParameterValue, RefSelector, ReleaseAgentId, ReleaseCommandKey, RuntimePolicy,
    TriggerPolicy,
};
use release_service::{
    BeginUpdateHook, CreateAttachment, CreateInstanceUpdate, ImportAgent, RecoverInstanceUpdate,
    ReleaseService, RemoveAttachment, ReviseInstance, SetAttachmentEnabled, UpdateRecoveryAction,
};
use runtime_types::RunId;
use secret_domain::{
    AgentSecretBindingId, DeliveryMode, ExecutionPhase, SecretAlias, SecretCommandKey,
    SecretGrantId, SecretId, SecretImportId, SecretName, SecretOwner, SecretSlotKey, SecretTarget,
    SecretUsePolicy, SecretValue, SecretVersionId,
};
use secret_service::{
    AcceptSecretImport, BindSecret, CreateSecret, GrantSecret, RotateSecret, SecretService,
};
use secret_store::LocalKeyProvider;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, sync::Arc};
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Clone)]
pub struct InternalCommandState {
    token_hash: [u8; 32],
    releases: Arc<ReleaseService>,
    secrets: Arc<SecretService<LocalKeyProvider>>,
    platform_policy: RuntimePolicy,
    platform_policy_version: String,
}

impl InternalCommandState {
    pub(crate) const fn new(
        token_hash: [u8; 32],
        releases: Arc<ReleaseService>,
        secrets: Arc<SecretService<LocalKeyProvider>>,
        platform_policy: RuntimePolicy,
        platform_policy_version: String,
    ) -> Self {
        Self {
            token_hash,
            releases,
            secrets,
            platform_policy,
            platform_policy_version,
        }
    }
}

pub fn router(state: InternalCommandState) -> Router {
    Router::new()
        .route("/internal/v1/commands", post(execute))
        .with_state(state)
}

#[derive(Deserialize)]
struct CommandEnvelope {
    actor_id: UserId,
    request_id: RequestId,
    #[serde(flatten)]
    command: InternalCommand,
}

#[derive(Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
enum InternalCommand {
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
        value: String,
    },
    RotateSecret {
        secret_id: SecretId,
        expected_active_version_id: SecretVersionId,
        value: String,
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

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RecoveryAction {
    Retry,
    Reject,
    Resume,
}

async fn execute(
    State(state): State<InternalCommandState>,
    headers: HeaderMap,
    Json(envelope): Json<CommandEnvelope>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    if !authorized(&headers, &state.token_hash) {
        return Err(rejection(StatusCode::UNAUTHORIZED));
    }
    let identity = AuthenticatedIdentity::new(
        envelope.actor_id,
        "hephaestus-web-mediator",
        envelope.actor_id.to_string(),
        json!({"mediator": "phoenix"}),
        envelope.request_id,
    );
    match dispatch(&state, &identity, envelope.command).await {
        Ok(value) => Ok(Json(value)),
        Err(error) => {
            tracing::warn!(
                actor_id = %identity.user_id,
                request_id = %identity.request_id,
                %error,
                "internal command rejected"
            );
            Err(rejection(StatusCode::UNPROCESSABLE_ENTITY))
        }
    }
}

// Keeping the command-to-domain mapping together makes the trusted internal
// boundary and its complete browser payload vocabulary directly auditable.
#[allow(clippy::too_many_lines)]
async fn dispatch(
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
            let instance_id = AgentInstanceId::new();
            let revision_id = AgentInstanceRevisionId::new();
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
            let attachment_id = AgentAttachmentId::new();
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
            let revision_id = AgentInstanceRevisionId::new();
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
            let update_id = AgentUpdateId::new();
            let revision_id = AgentInstanceRevisionId::new();
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
            let hook_run_id = RunId::new();
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
            let secret_id = SecretId::new();
            let version_id = SecretVersionId::new();
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
                        value: SecretValue::new(value.into_bytes())?,
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
            let version_id = SecretVersionId::new();
            state
                .secrets
                .rotate(
                    identity,
                    RotateSecret {
                        command_key: secret_key(identity, "rotate_secret", version_id.as_uuid()),
                        secret_id,
                        expected_active_version_id,
                        new_version_id: version_id,
                        value: SecretValue::new(value.into_bytes())?,
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
            let grant_id = SecretGrantId::new();
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
            let import_id = SecretImportId::new();
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
            let binding_id = AgentSecretBindingId::new();
            let revision_id = AgentInstanceRevisionId::new();
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

fn authorized(headers: &HeaderMap, expected_hash: &[u8; 32]) -> bool {
    let Some(value) = headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
    else {
        return false;
    };
    let actual: [u8; 32] = Sha256::digest(value.as_bytes()).into();
    let mut difference = 0_u8;
    for (actual, expected) in actual.iter().zip(expected_hash) {
        difference |= actual ^ expected;
    }
    difference == 0
}

fn release_key(
    identity: &AuthenticatedIdentity,
    operation: &str,
    aggregate_id: Uuid,
) -> ReleaseCommandKey {
    ReleaseCommandKey::derive(
        operation,
        &[
            identity.request_id.as_uuid().as_bytes(),
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
            identity.request_id.as_uuid().as_bytes(),
            aggregate_id.as_bytes(),
        ],
    )
}

fn rejection(status: StatusCode) -> (StatusCode, Json<Value>) {
    (status, Json(json!({"error": "command rejected"})))
}

#[cfg(test)]
mod tests {
    use super::{CommandEnvelope, InternalCommand, authorized};
    use axum::http::{HeaderMap, HeaderValue, header::AUTHORIZATION};
    use serde_json::json;
    use sha2::{Digest, Sha256};

    #[test]
    fn bearer_authentication_is_exact_and_value_free() {
        let expected: [u8; 32] = Sha256::digest(b"correct-internal-token").into();
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_static("Bearer correct-internal-token"),
        );
        assert!(authorized(&headers, &expected));
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_static("Bearer incorrect-internal-token"),
        );
        assert!(!authorized(&headers, &expected));
    }

    #[test]
    fn browser_secret_command_shapes_deserialize_exactly() {
        let actor = "10000000-0000-4000-8000-000000000001";
        let request = "10000000-0000-4000-8000-000000000002";
        let object = "10000000-0000-4000-8000-000000000003";

        let create: CommandEnvelope = serde_json::from_value(json!({
            "actor_id": actor,
            "request_id": request,
            "operation": "create_secret",
            "owner": {"type": "organization", "id": object},
            "name": "organization_token",
            "allowed_delivery_modes": ["raw"],
            "value": "transient"
        }))
        .expect("create-secret browser shape");
        assert!(matches!(
            create.command,
            InternalCommand::CreateSecret { .. }
        ));

        let accept: CommandEnvelope = serde_json::from_value(json!({
            "actor_id": actor,
            "request_id": request,
            "operation": "accept_secret_import",
            "grant_id": object,
            "target": {"type": "project", "id": request},
            "alias": "org_token"
        }))
        .expect("accept-import browser shape");
        assert!(matches!(
            accept.command,
            InternalCommand::AcceptSecretImport { .. }
        ));

        let bind: CommandEnvelope = serde_json::from_value(json!({
            "actor_id": actor,
            "request_id": request,
            "operation": "bind_secret",
            "instance_id": object,
            "expected_revision_id": request,
            "import_id": actor,
            "slot": "raw_token",
            "mode": "raw",
            "phases": ["normal"],
            "attachment_ids": [object],
            "destinations": []
        }))
        .expect("bind-secret browser shape");
        assert!(matches!(bind.command, InternalCommand::BindSecret { .. }));
    }
}
