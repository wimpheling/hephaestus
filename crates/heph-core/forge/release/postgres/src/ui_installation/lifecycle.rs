use super::AttemptError;
use super::ledger::{
    append_owner_event, find_existing_command, lock_owner, replay_lifecycle_or_conflict,
};
use super::owner::{
    installation_target, lifecycle_name, lifecycle_state, load_installation_owner,
    lock_installation, receipt_scope, require_owner_management, same_owner,
};
use super::persistence::map_command_insert_error;
use crate::ReleaseService;
use identity_domain::{AuthenticatedIdentity, actor_idempotency_id};
use release_domain::{
    UiInstallationCommandIdentity, UiInstallationGenerationId, UiInstallationId,
    UiInstallationInputDigest, UiInstallationOperation, UiInstallationState,
};
use release_service::{UiInstallationError, UiInstallationLifecycleResult};

impl ReleaseService {
    // Keep the lifecycle transaction ordering visible beside the command
    // boundary: owner lock, authorization, immutable ledger, then outbox.
    #[allow(clippy::too_many_lines)]
    // Shared by the command facade in the parent module.
    #[allow(clippy::redundant_pub_crate)]
    pub(super) async fn mutate_ui_installation(
        &self,
        identity: &AuthenticatedIdentity,
        installation_id: UiInstallationId,
        caller_key: release_domain::UiInstallationCallerKey,
        expected_generation_id: Option<UiInstallationGenerationId>,
        operation: UiInstallationOperation,
    ) -> Result<UiInstallationLifecycleResult, AttemptError> {
        let next_state = match operation {
            UiInstallationOperation::Disable => UiInstallationState::Disabled,
            UiInstallationOperation::Remove => UiInstallationState::Removed,
            _ => return Err(UiInstallationError::Unavailable.into()),
        };
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
            UiInstallationOperation::Disable => {
                UiInstallationInputDigest::disable(installation_id, expected_generation_id)
            }
            UiInstallationOperation::Remove => {
                UiInstallationInputDigest::remove(installation_id, expected_generation_id)
            }
            _ => unreachable!("lifecycle mutation only accepts disable/remove"),
        };
        if let Some(existing) = find_existing_command(
            &mut tx,
            identity.user_id.as_uuid(),
            operation,
            caller_key.as_str(),
        )
        .await
        .map_err(|_| UiInstallationError::Unavailable)?
        {
            return replay_lifecycle_or_conflict(
                &existing,
                command_key,
                input_hash,
                identity.user_id.as_uuid(),
                installation_id,
                next_state,
                target,
            );
        }
        // Disabled -> disabled is intentionally valid: a fresh caller key is
        // a new effective command and therefore emits its own owner event.
        if locked.lifecycle == "removed"
            || !lifecycle_state(&locked.lifecycle)
                .is_some_and(|state| state.can_transition_to(next_state))
        {
            return Err(UiInstallationError::InvalidTransition.into());
        }
        if expected_generation_id
            .is_some_and(|expected| expected.as_uuid() != locked.current_generation_id)
        {
            return Err(UiInstallationError::GenerationConflict.into());
        }

        let result_lifecycle = lifecycle_name(next_state);
        sqlx::query(
            "UPDATE ui_installations
             SET lifecycle = $2, removed_at = CASE WHEN $2 = 'removed' THEN now() ELSE NULL END,
                 updated_at = now()
             WHERE id = $1",
        )
        .bind(installation_id.as_uuid())
        .bind(result_lifecycle)
        .execute(&mut *tx)
        .await
        .map_err(|_| UiInstallationError::Unavailable)?;
        let command_insert = sqlx::query(
            "INSERT INTO ui_installation_commands
             (command_key, caller_idempotency_key, operation, installation_id, actor_id,
              request_id, input_hash, expected_generation_id, result_generation_id,
              result_lifecycle)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
        )
        .bind(command_key.as_bytes().as_slice())
        .bind(caller_key.as_str())
        .bind(operation.as_str())
        .bind(installation_id.as_uuid())
        .bind(identity.user_id.as_uuid())
        .bind(identity.request_id.as_uuid())
        .bind(input_hash.as_bytes().as_slice())
        .bind(expected_generation_id.map(release_domain::UiInstallationGenerationId::as_uuid))
        .bind(locked.current_generation_id)
        .bind(result_lifecycle)
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
        Ok(UiInstallationLifecycleResult {
            installation_id,
            receipt_scope: receipt_scope(target),
            generation_id: UiInstallationGenerationId::from_uuid(locked.current_generation_id),
            state: next_state,
            idempotency_id: idempotency_id.as_uuid(),
        })
    }
}
