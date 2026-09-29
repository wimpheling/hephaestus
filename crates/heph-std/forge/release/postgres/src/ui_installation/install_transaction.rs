use super::bindings::{resolve_repository_git_access, resolve_ui_bindings};
use super::ledger::lock_owner;
use super::ledger::{
    active_installation, append_owner_event, find_existing_command, replay_or_conflict,
};
use super::persistence::{insert_installation, map_authorization_error, map_command_insert_error};
use super::{AttemptError, UiBindingResolutionError};
use crate::ReleaseService;
use authz_domain::{ObjectRef, ObjectType, Permission};
use identity_domain::{AuthenticatedIdentity, actor_idempotency_id};
use release_domain::{
    UiInstallationCommandIdentity, UiInstallationGenerationId, UiInstallationId,
    UiInstallationInputDigest, UiInstallationState,
};
use release_service::{InstallUi, InstallUiResult, UiInstallationError};

impl ReleaseService {
    // Keep the ordered transaction in one function so its rollback and
    // bounded idempotency retry boundary remain directly auditable.
    // The nested transaction deliberately keeps all owner and binding checks
    // together; splitting it would obscure its rollback boundary.
    #[allow(clippy::cognitive_complexity, clippy::too_many_lines)]
    // Shared by the command facade in the parent module.
    #[allow(clippy::redundant_pub_crate)]
    pub(super) async fn install_ui_once(
        &self,
        identity: &AuthenticatedIdentity,
        command: &InstallUi,
        static_zero_api_only: bool,
    ) -> Result<InstallUiResult, AttemptError> {
        let mut tx = authz_postgres::begin_actor_transaction(&self.pool, identity)
            .await
            .map_err(|_| UiInstallationError::Unavailable)?;
        let owner = lock_owner(&mut tx, command.target)
            .await
            .map_err(|_| UiInstallationError::Unavailable)?
            .ok_or(UiInstallationError::Unavailable)?;
        if command
            .expected_organization_id
            .is_some_and(|expected| expected.as_uuid() != owner.organization)
        {
            return Err(UiInstallationError::OrganizationMismatch.into());
        }
        match command.target {
            release_domain::UiInstallationTarget::Organization(_) => {
                self.require(
                    &mut tx,
                    identity,
                    Permission::CanManage,
                    ObjectRef::new(ObjectType::Organization, owner.organization),
                )
                .await
                .map_err(|error| map_authorization_error(&error))?;
            }
            release_domain::UiInstallationTarget::Project(_)
            | release_domain::UiInstallationTarget::Repository(_) => {
                let project_id = owner.project.ok_or(UiInstallationError::Unavailable)?;
                self.require(
                    &mut tx,
                    identity,
                    Permission::CanManage,
                    ObjectRef::new(ObjectType::Project, project_id),
                )
                .await
                .map_err(|error| map_authorization_error(&error))?;
                if let Some(repository_id) = owner.repository {
                    self.require(
                        &mut tx,
                        identity,
                        Permission::CanWrite,
                        ObjectRef::new(ObjectType::Repository, repository_id),
                    )
                    .await
                    .map_err(|error| map_authorization_error(&error))?;
                }
            }
        }

        let command_identity = UiInstallationCommandIdentity::new(
            identity.user_id.as_uuid(),
            release_domain::UiInstallationOperation::Install,
            command.caller_key.clone(),
        );
        let command_key = command_identity.command_key();
        let input_hash = UiInstallationInputDigest::install_with_expected_organization_and_git_ack(
            command.target,
            command.release_id,
            &command.ui_key,
            command.expected_organization_id,
            command.acknowledge_repository_git_access,
        );
        // The target owner is the current authority for replay. A stored receipt
        // remains durable even when its source gateway or release permissions
        // later change; serving admission rechecks those mutable bindings.
        if let Some(existing) = find_existing_command(
            &mut tx,
            identity.user_id.as_uuid(),
            release_domain::UiInstallationOperation::Install,
            command.caller_key.as_str(),
        )
        .await
        .map_err(|_| UiInstallationError::Unavailable)?
        {
            return replay_or_conflict(
                &existing,
                command_key,
                input_hash,
                identity.user_id.as_uuid(),
            );
        }

        // No receipt exists, so all source and serving permissions are current
        // requirements for this new installation.
        self.require(
            &mut tx,
            identity,
            Permission::CanUse,
            ObjectRef::new(ObjectType::Release, command.release_id.as_uuid()),
        )
        .await
        .map_err(|error| map_authorization_error(&error))?;
        let bindings = resolve_ui_bindings(
            &mut tx,
            identity.user_id.as_uuid(),
            command.release_id,
            owner.organization,
            command.target.scope_name(),
            &command.ui_key,
            static_zero_api_only,
        )
        .await
        .map_err(|error| match error {
            UiBindingResolutionError::Persistence => UiInstallationError::Unavailable,
            UiBindingResolutionError::Invalid => UiInstallationError::InvalidOrUnsupported,
        })?;
        let repository_git_access = resolve_repository_git_access(
            &mut tx,
            command.release_id,
            &command.ui_key,
            command.target.scope_name(),
            matches!(
                command.target,
                release_domain::UiInstallationTarget::Repository(_)
            ),
            command.acknowledge_repository_git_access,
            None,
        )
        .await
        .map_err(|error| match error {
            UiBindingResolutionError::Persistence => UiInstallationError::Unavailable,
            UiBindingResolutionError::Invalid => UiInstallationError::InvalidOrUnsupported,
        })?;
        for binding in &bindings {
            self.require(
                &mut tx,
                identity,
                Permission::CanUse,
                ObjectRef::new(ObjectType::ReleaseAgent, binding.release_agent_id),
            )
            .await
            .map_err(|error| map_authorization_error(&error))?;
        }
        let organization_id = matches!(
            command.target,
            release_domain::UiInstallationTarget::Organization(_)
        )
        .then_some(owner.organization);
        let already_active = active_installation(
            &mut tx,
            organization_id,
            owner.project,
            owner.repository,
            &command.ui_key,
        )
        .await
        .map_err(|_| UiInstallationError::Unavailable)?;
        if already_active {
            return Err(UiInstallationError::AlreadyInstalled.into());
        }

        let installation_id = UiInstallationId::new();
        let generation_id = UiInstallationGenerationId::new();
        insert_installation(
            &mut tx,
            installation_id,
            generation_id,
            organization_id,
            owner.project,
            owner.repository,
            command.target.scope_name(),
            &command.ui_key,
            identity.user_id.as_uuid(),
        )
        .await?;
        sqlx::query(
            "INSERT INTO ui_installation_generations
             (id, installation_id, generation_no, release_id, ui_key, ui_scope,
              repository_git_access)
             VALUES ($1, $2, 1, $3, $4, $5, $6)",
        )
        .bind(generation_id.as_uuid())
        .bind(installation_id.as_uuid())
        .bind(command.release_id.as_uuid())
        .bind(command.ui_key.as_str())
        .bind(command.target.scope_name())
        .bind(repository_git_access)
        .execute(&mut *tx)
        .await
        .map_err(|_| UiInstallationError::Unavailable)?;
        for binding in &bindings {
            sqlx::query(
                "INSERT INTO ui_installation_bindings
                 (installation_id, generation_id, binding_kind, binding_key,
                  release_id, ui_key, gateway_id, gateway_revision_id,
                  release_agent_id, gateway_name, method, route, exposure)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12,
                         'heph_authenticated')",
            )
            .bind(installation_id.as_uuid())
            .bind(generation_id.as_uuid())
            .bind(binding.binding_kind)
            .bind(&binding.binding_key)
            .bind(command.release_id.as_uuid())
            .bind(command.ui_key.as_str())
            .bind(binding.gateway_id)
            .bind(binding.gateway_revision_id)
            .bind(binding.release_agent_id)
            .bind(&binding.gateway_name)
            .bind(&binding.method)
            .bind(&binding.route)
            .execute(&mut *tx)
            .await
            .map_err(|_| UiInstallationError::Unavailable)?;
        }
        let command_insert = sqlx::query(
            "INSERT INTO ui_installation_commands
             (command_key, caller_idempotency_key, operation, installation_id, actor_id,
              request_id, input_hash, result_generation_id, result_lifecycle)
             VALUES ($1, $2, 'install', $3, $4, $5, $6, $7, 'enabled')",
        )
        .bind(command_key.as_bytes().as_slice())
        .bind(command.caller_key.as_str())
        .bind(installation_id.as_uuid())
        .bind(identity.user_id.as_uuid())
        .bind(identity.request_id.as_uuid())
        .bind(input_hash.as_bytes().as_slice())
        .bind(generation_id.as_uuid())
        .execute(&mut *tx)
        .await;
        if let Err(error) = command_insert {
            let mapped = map_command_insert_error(&error);
            if matches!(mapped, AttemptError::RetryLedgerRace) {
                tx.rollback()
                    .await
                    .map_err(|_| UiInstallationError::Unavailable)?;
            }
            return Err(mapped);
        }

        let idempotency_id = actor_idempotency_id(
            identity.user_id.as_uuid().as_bytes(),
            command_key.as_bytes(),
        );
        sqlx::query("SELECT set_config('hephaestus.occurrence_id', $1, true)")
            .bind(idempotency_id.to_string())
            .execute(&mut *tx)
            .await
            .map_err(|_| UiInstallationError::Unavailable)?;
        append_owner_event(&mut tx, idempotency_id.as_uuid(), &owner)
            .await
            .map_err(|_| UiInstallationError::Unavailable)?;
        tx.commit()
            .await
            .map_err(|_| UiInstallationError::Unavailable)?;
        Ok(InstallUiResult {
            installation_id,
            generation_id,
            state: UiInstallationState::Enabled,
            idempotency_id: idempotency_id.as_uuid(),
        })
    }
}
