//! Secret command execution.

use super::{
    ids::{declared_rule_id, secret_key, stable_id},
    state::InternalCommandState,
    types::InternalCommand,
};
use identity_domain::AuthenticatedIdentity;
use release_domain::AgentInstanceRevisionId;
use secret_application::{
    AcceptSecretImport, BindSecret, CreateSecret, DeclareBrokeredHttpsRule, GrantSecret,
    RotateSecret,
};
use secret_domain::{
    AgentSecretBindingId, SecretGrantId, SecretId, SecretImportId, SecretValue, SecretVersionId,
};
use serde_json::{Value, json};
use std::error::Error;

// Keep secret command mapping together so every sensitive operation remains auditable at one boundary.
#[allow(clippy::too_many_lines)]
pub(super) async fn dispatch_secret(
    state: &InternalCommandState,
    identity: &AuthenticatedIdentity,
    command: InternalCommand,
) -> Result<Value, Box<dyn Error>> {
    match command {
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
            requested_rule_id,
        } => {
            let rule_id = declared_rule_id(identity, requested_rule_id);
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

        _ => unreachable!("non-secret command routed to secret dispatcher"),
    }
}
