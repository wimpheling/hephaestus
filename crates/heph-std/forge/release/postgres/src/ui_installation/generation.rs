use super::bindings::{resolve_repository_git_access, resolve_ui_bindings};
use super::ledger::{
    append_owner_event, find_existing_command, insert_generation_bindings, lock_owner,
    replay_generation_or_conflict,
};
use super::owner::{
    installation_target, load_installation_owner, lock_installation, receipt_scope,
    require_owner_management, same_owner,
};
use super::persistence::{map_authorization_error, map_command_insert_error};
use super::{AttemptError, UiBindingResolutionError};
use crate::ReleaseService;
use authz_domain::{ObjectRef, ObjectType, Permission};
use identity_domain::{AuthenticatedIdentity, actor_idempotency_id};
use release_domain::{
    ReleaseId, UiInstallationCommandIdentity, UiInstallationGenerationId, UiInstallationId,
    UiInstallationInputDigest, UiInstallationOperation, UiInstallationState,
};
use release_service::{UiInstallationError, UiInstallationGenerationResult};

impl ReleaseService {
    // Keep generation replacement in one transaction: owner lock, current
    // owner authorization, receipt replay, CAS, current source binding
    // resolution, immutable rows, pointer update, then one owner outbox event.
    #[allow(clippy::too_many_lines)]
    // Keep the ordered mutation inputs explicit so activation and rollback
    // share one auditable transaction path.
    #[allow(clippy::too_many_arguments)]
    // Shared by the command facade in the parent module.
    #[allow(clippy::redundant_pub_crate)]
    pub(super) async fn mutate_ui_generation(
        &self,
        identity: &AuthenticatedIdentity,
        installation_id: UiInstallationId,
        caller_key: release_domain::UiInstallationCallerKey,
        expected_generation_id: Option<UiInstallationGenerationId>,
        release_id: ReleaseId,
        ui_key: release_domain::ui::UiKey,
        operation: UiInstallationOperation,
    ) -> Result<UiInstallationGenerationResult, AttemptError> {
        let mut tx = authz_postgres::begin_actor_transaction(&self.pool, identity)
            .await
            .map_err(|_| UiInstallationError::Unavailable)?;
        let initial = load_installation_owner(&mut tx, installation_id)
            .await
            .map_err(|_| UiInstallationError::Unavailable)?
            .ok_or(UiInstallationError::Unavailable)?;
        let target = installation_target(&initial).ok_or(UiInstallationError::Unavailable)?;
        let owner = lock_owner(&mut tx, target)
            .await
            .map_err(|_| UiInstallationError::Unavailable)?
            .ok_or(UiInstallationError::Unavailable)?;
        require_owner_management(self, &mut tx, identity, target, &owner).await?;
        let locked = lock_installation(&mut tx, installation_id)
            .await
            .map_err(|_| UiInstallationError::Unavailable)?
            .ok_or(UiInstallationError::Unavailable)?;
        if !same_owner(&initial, &locked) {
            return Err(UiInstallationError::Unavailable.into());
        }

        let command_identity = UiInstallationCommandIdentity::new(
            identity.user_id.as_uuid(),
            operation,
            caller_key.clone(),
        );
        let command_key = command_identity.command_key();
        let input_hash = match operation {
            UiInstallationOperation::Activate => UiInstallationInputDigest::activate(
                installation_id,
                expected_generation_id,
                release_id,
                &ui_key,
            ),
            UiInstallationOperation::Rollback => UiInstallationInputDigest::rollback(
                installation_id,
                expected_generation_id,
                release_id,
                &ui_key,
            ),
            _ => return Err(UiInstallationError::Unavailable.into()),
        };
        // Current owner authority is required before replay. A matching receipt
        // returns its original generation without rereading mutable source or
        // gateway state; serving admission remains a separate current check.
        if let Some(existing) = find_existing_command(
            &mut tx,
            identity.user_id.as_uuid(),
            operation,
            caller_key.as_str(),
        )
        .await
        .map_err(|_| UiInstallationError::Unavailable)?
        {
            return replay_generation_or_conflict(
                &existing,
                command_key,
                input_hash,
                identity.user_id.as_uuid(),
                installation_id,
                target,
            );
        }
        if locked.lifecycle == "removed" {
            return Err(UiInstallationError::InvalidTransition.into());
        }
        if locked.ui_key != ui_key.as_str()
            || initial.ui_key != ui_key.as_str()
            || locked.scope != target.scope_name()
        {
            return Err(UiInstallationError::InvalidOrUnsupported.into());
        }
        if expected_generation_id
            .is_some_and(|expected| expected.as_uuid() != locked.current_generation_id)
        {
            return Err(UiInstallationError::GenerationConflict.into());
        }

        // No receipt exists, so all current release, source, gateway, and
        // release-agent permissions are required for the replacement.
        self.require(
            &mut tx,
            identity,
            Permission::CanUse,
            ObjectRef::new(ObjectType::Release, release_id.as_uuid()),
        )
        .await
        .map_err(|error| map_authorization_error(&error))?;
        let bindings = resolve_ui_bindings(
            &mut tx,
            identity.user_id.as_uuid(),
            release_id,
            owner.organization,
            target.scope_name(),
            &ui_key,
            false,
        )
        .await
        .map_err(|error| match error {
            UiBindingResolutionError::Persistence => UiInstallationError::Unavailable,
            UiBindingResolutionError::Invalid => UiInstallationError::InvalidOrUnsupported,
        })?;
        let repository_git_access = resolve_repository_git_access(
            &mut tx,
            release_id,
            &ui_key,
            target.scope_name(),
            matches!(target, release_domain::UiInstallationTarget::Repository(_)),
            false,
            Some(locked.current_generation_id),
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
        let generation_no = locked
            .current_generation_no
            .checked_add(1)
            .ok_or(UiInstallationError::Unavailable)?;
        let generation_id = UiInstallationGenerationId::new();
        sqlx::query(
            "INSERT INTO ui_installation_generations
             (id, installation_id, generation_no, release_id, ui_key, ui_scope,
              repository_git_access)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(generation_id.as_uuid())
        .bind(installation_id.as_uuid())
        .bind(generation_no)
        .bind(release_id.as_uuid())
        .bind(ui_key.as_str())
        .bind(target.scope_name())
        .bind(repository_git_access)
        .execute(&mut *tx)
        .await
        .map_err(|_| UiInstallationError::Unavailable)?;
        insert_generation_bindings(
            &mut tx,
            installation_id,
            generation_id,
            release_id,
            &ui_key,
            &bindings,
        )
        .await?;
        sqlx::query(
            "UPDATE ui_installations
             SET current_generation_id = $2, lifecycle = 'enabled', removed_at = NULL,
                 updated_at = now()
             WHERE id = $1",
        )
        .bind(installation_id.as_uuid())
        .bind(generation_id.as_uuid())
        .execute(&mut *tx)
        .await
        .map_err(|_| UiInstallationError::Unavailable)?;
        let command_insert = sqlx::query(
            "INSERT INTO ui_installation_commands
             (command_key, caller_idempotency_key, operation, installation_id, actor_id,
              request_id, input_hash, expected_generation_id, result_generation_id,
              result_lifecycle)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 'enabled')",
        )
        .bind(command_key.as_bytes().as_slice())
        .bind(caller_key.as_str())
        .bind(operation.as_str())
        .bind(installation_id.as_uuid())
        .bind(identity.user_id.as_uuid())
        .bind(identity.request_id.as_uuid())
        .bind(input_hash.as_bytes().as_slice())
        .bind(expected_generation_id.map(UiInstallationGenerationId::as_uuid))
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
        Ok(UiInstallationGenerationResult {
            installation_id,
            receipt_scope: receipt_scope(target),
            generation_id,
            state: UiInstallationState::Enabled,
            idempotency_id: idempotency_id.as_uuid(),
        })
    }
}
