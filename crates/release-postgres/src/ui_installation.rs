//! `PostgreSQL` adapter for static UI installation and lifecycle commands.

use authz_domain::{ObjectRef, ObjectType, Permission};
use forge_domain::{OrganizationId, ProjectId, RepositoryId};
use identity_domain::{AuthenticatedIdentity, actor_idempotency_id};
use release_domain::{
    ReleaseCommandKey, ReleaseId, UiInstallationCommandIdentity, UiInstallationGenerationId,
    UiInstallationId, UiInstallationInputDigest, UiInstallationOperation, UiInstallationState,
};
use release_service::{
    ActivateUiInstallation, DisableUiInstallation, InstallStaticUi, InstallStaticUiResult,
    InstallUi, InstallUiResult, RemoveUiInstallation, RollbackUiInstallation, UiInstallationError,
    UiInstallationGenerationResult, UiInstallationLifecycleResult,
};
use sqlx::{FromRow, Postgres, Transaction};
use uuid::Uuid;

use super::ReleaseService;

#[derive(Debug, FromRow)]
struct OwnerRow {
    #[sqlx(rename = "project_id")]
    project: Option<Uuid>,
    #[sqlx(rename = "organization_id")]
    organization: Uuid,
    #[sqlx(rename = "repository_id")]
    repository: Option<Uuid>,
}

#[derive(Debug, FromRow)]
struct InstallationOwnerRow {
    #[sqlx(rename = "project_id")]
    project: Option<Uuid>,
    #[sqlx(rename = "organization_id")]
    organization: Uuid,
    #[sqlx(rename = "repository_id")]
    repository: Option<Uuid>,
    scope: String,
    ui_key: String,
}

#[derive(Debug, FromRow)]
struct LockedInstallationRow {
    #[sqlx(rename = "project_id")]
    project: Option<Uuid>,
    #[sqlx(rename = "organization_id")]
    organization: Uuid,
    #[sqlx(rename = "repository_id")]
    repository: Option<Uuid>,
    scope: String,
    ui_key: String,
    lifecycle: String,
    current_generation_id: Uuid,
    current_generation_no: i64,
}

#[derive(Debug, FromRow)]
struct ExistingInstallCommand {
    command_key: Vec<u8>,
    input_hash: Vec<u8>,
    installation_id: Uuid,
    result_generation_id: Uuid,
    result_lifecycle: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AttemptError {
    Public(UiInstallationError),
    RetryLedgerRace,
}

impl From<UiInstallationError> for AttemptError {
    fn from(error: UiInstallationError) -> Self {
        Self::Public(error)
    }
}

impl ReleaseService {
    /// Installs one published UI and pins every declared gateway binding.
    ///
    /// The owner row is locked before authorization, replay lookup, or
    /// generation creation. Gateway rows and source parent rows are then
    /// resolved under the same transaction, so an invalid binding rolls back
    /// the complete installation receipt.
    ///
    /// # Errors
    ///
    /// Returns a redacted failure for authorization, unsupported publication
    /// content, unavailable gateway bindings, idempotency conflict, an
    /// existing active installation, or unavailable persistence.
    pub async fn install_ui(
        &self,
        identity: &AuthenticatedIdentity,
        command: InstallUi,
    ) -> Result<InstallUiResult, UiInstallationError> {
        for attempt in 0..2 {
            match self.install_ui_once(identity, &command, false).await {
                Err(AttemptError::RetryLedgerRace) if attempt == 0 => {}
                Err(AttemptError::RetryLedgerRace) => return Err(UiInstallationError::Unavailable),
                Err(AttemptError::Public(error)) => return Err(error),
                Ok(result) => return Ok(result),
            }
        }
        Err(UiInstallationError::Unavailable)
    }

    /// Installs one published static UI with no API or managed bindings.
    ///
    /// This compatibility wrapper delegates to the generalized installation
    /// transaction while retaining the original static zero-binding contract.
    ///
    /// # Errors
    ///
    /// Returns a redacted, transport-neutral failure for authorization,
    /// unsupported publication content, idempotency conflict, an existing
    /// active installation, or unavailable persistence.
    ///
    /// The owner row is locked before authorization, command replay, or
    /// installation lookup. A second bounded attempt handles a same-actor
    /// caller-key race on a different owner row by rereading the committed
    /// immutable ledger; it never retries a failed transaction in place.
    ///
    /// # Errors
    ///
    /// Returns a redacted, transport-neutral failure for authorization,
    /// unsupported publication content, idempotency conflict, an existing
    /// active installation, or unavailable persistence.
    pub async fn install_static_ui(
        &self,
        identity: &AuthenticatedIdentity,
        command: InstallStaticUi,
    ) -> Result<InstallStaticUiResult, UiInstallationError> {
        let command = InstallUi {
            caller_key: command.caller_key,
            target: command.target,
            release_id: command.release_id,
            ui_key: command.ui_key,
            expected_organization_id: None,
        };
        for attempt in 0..2 {
            match self.install_ui_once(identity, &command, true).await {
                Err(AttemptError::RetryLedgerRace) if attempt == 0 => {}
                Err(AttemptError::RetryLedgerRace) => return Err(UiInstallationError::Unavailable),
                Err(AttemptError::Public(error)) => return Err(error),
                Ok(result) => {
                    return Ok(InstallStaticUiResult {
                        installation_id: result.installation_id,
                        generation_id: result.generation_id,
                        state: result.state,
                        idempotency_id: result.idempotency_id,
                    });
                }
            }
        }
        Err(UiInstallationError::Unavailable)
    }

    /// Activates a new immutable generation for an installation.
    ///
    /// # Errors
    ///
    /// Returns a redacted failure for unavailable persistence, denied current
    /// owner or source authority, stale generation CAS, invalid publication,
    /// terminal installation state, or changed replay input.
    pub async fn activate_ui_installation(
        &self,
        identity: &AuthenticatedIdentity,
        command: ActivateUiInstallation,
    ) -> Result<UiInstallationGenerationResult, UiInstallationError> {
        for attempt in 0..2 {
            match self
                .mutate_ui_generation(
                    identity,
                    command.installation_id,
                    command.caller_key.clone(),
                    command.expected_generation_id,
                    command.release_id,
                    command.ui_key.clone(),
                    UiInstallationOperation::Activate,
                )
                .await
            {
                Err(AttemptError::RetryLedgerRace) if attempt == 0 => {}
                Err(AttemptError::RetryLedgerRace) => return Err(UiInstallationError::Unavailable),
                Err(AttemptError::Public(error)) => return Err(error),
                Ok(result) => return Ok(result),
            }
        }
        Err(UiInstallationError::Unavailable)
    }

    /// Rolls an installation back to a selected release as a new immutable
    /// generation; historical generations remain addressable.
    ///
    /// # Errors
    ///
    /// Returns a redacted failure for unavailable persistence, denied current
    /// owner or source authority, stale generation CAS, invalid publication,
    /// terminal installation state, or changed replay input.
    pub async fn rollback_ui_installation(
        &self,
        identity: &AuthenticatedIdentity,
        command: RollbackUiInstallation,
    ) -> Result<UiInstallationGenerationResult, UiInstallationError> {
        for attempt in 0..2 {
            match self
                .mutate_ui_generation(
                    identity,
                    command.installation_id,
                    command.caller_key.clone(),
                    command.expected_generation_id,
                    command.release_id,
                    command.ui_key.clone(),
                    UiInstallationOperation::Rollback,
                )
                .await
            {
                Err(AttemptError::RetryLedgerRace) if attempt == 0 => {}
                Err(AttemptError::RetryLedgerRace) => return Err(UiInstallationError::Unavailable),
                Err(AttemptError::Public(error)) => return Err(error),
                Ok(result) => return Ok(result),
            }
        }
        Err(UiInstallationError::Unavailable)
    }

    /// Disables an installation while retaining its current generation and
    /// immutable history. Source-release permissions are deliberately not
    /// consulted: current ownership authority controls this lifecycle change.
    ///
    /// # Errors
    ///
    /// Returns a redacted failure for unavailable persistence, denied current
    /// owner authority, stale generation CAS, invalid lifecycle state, or a
    /// changed replay input.
    pub async fn disable_ui_installation(
        &self,
        identity: &AuthenticatedIdentity,
        command: DisableUiInstallation,
    ) -> Result<UiInstallationLifecycleResult, UiInstallationError> {
        for attempt in 0..2 {
            match self
                .mutate_ui_installation(
                    identity,
                    command.installation_id,
                    command.caller_key.clone(),
                    command.expected_generation_id,
                    UiInstallationOperation::Disable,
                )
                .await
            {
                Err(AttemptError::RetryLedgerRace) if attempt == 0 => {}
                Err(AttemptError::RetryLedgerRace) => return Err(UiInstallationError::Unavailable),
                Err(AttemptError::Public(error)) => return Err(error),
                Ok(result) => return Ok(result),
            }
        }
        Err(UiInstallationError::Unavailable)
    }

    /// Terminally removes an installation while retaining its current
    /// generation and immutable history. The owner/key can then be reused by
    /// a new installation identity.
    ///
    /// # Errors
    ///
    /// Returns a redacted failure for unavailable persistence, denied current
    /// owner authority, stale generation CAS, invalid lifecycle state, or a
    /// changed replay input.
    pub async fn remove_ui_installation(
        &self,
        identity: &AuthenticatedIdentity,
        command: RemoveUiInstallation,
    ) -> Result<UiInstallationLifecycleResult, UiInstallationError> {
        for attempt in 0..2 {
            match self
                .mutate_ui_installation(
                    identity,
                    command.installation_id,
                    command.caller_key.clone(),
                    command.expected_generation_id,
                    UiInstallationOperation::Remove,
                )
                .await
            {
                Err(AttemptError::RetryLedgerRace) if attempt == 0 => {}
                Err(AttemptError::RetryLedgerRace) => return Err(UiInstallationError::Unavailable),
                Err(AttemptError::Public(error)) => return Err(error),
                Ok(result) => return Ok(result),
            }
        }
        Err(UiInstallationError::Unavailable)
    }

    // Keep generation replacement in one transaction: owner lock, current
    // owner authorization, receipt replay, CAS, current source binding
    // resolution, immutable rows, pointer update, then one owner outbox event.
    #[allow(clippy::too_many_lines)]
    // Keep the ordered mutation inputs explicit so activation and rollback
    // share one auditable transaction path.
    #[allow(clippy::too_many_arguments)]
    async fn mutate_ui_generation(
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
             (id, installation_id, generation_no, release_id, ui_key, ui_scope)
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(generation_id.as_uuid())
        .bind(installation_id.as_uuid())
        .bind(generation_no)
        .bind(release_id.as_uuid())
        .bind(ui_key.as_str())
        .bind(target.scope_name())
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
            generation_id,
            state: UiInstallationState::Enabled,
            idempotency_id: idempotency_id.as_uuid(),
        })
    }

    // Keep the lifecycle transaction ordering visible beside the command
    // boundary: owner lock, authorization, immutable ledger, then outbox.
    #[allow(clippy::too_many_lines)]
    async fn mutate_ui_installation(
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
            generation_id: UiInstallationGenerationId::from_uuid(locked.current_generation_id),
            state: next_state,
            idempotency_id: idempotency_id.as_uuid(),
        })
    }

    // Keep the ordered transaction in one function so its rollback and
    // bounded idempotency retry boundary remain directly auditable.
    // The nested transaction deliberately keeps all owner and binding checks
    // together; splitting it would obscure its rollback boundary.
    #[allow(clippy::cognitive_complexity, clippy::too_many_lines)]
    async fn install_ui_once(
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
        let input_hash = UiInstallationInputDigest::install_with_expected_organization(
            command.target,
            command.release_id,
            &command.ui_key,
            command.expected_organization_id,
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
             (id, installation_id, generation_no, release_id, ui_key, ui_scope)
             VALUES ($1, $2, 1, $3, $4, $5)",
        )
        .bind(generation_id.as_uuid())
        .bind(installation_id.as_uuid())
        .bind(command.release_id.as_uuid())
        .bind(command.ui_key.as_str())
        .bind(command.target.scope_name())
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

async fn load_installation_owner(
    tx: &mut Transaction<'_, Postgres>,
    installation_id: UiInstallationId,
) -> Result<Option<InstallationOwnerRow>, sqlx::Error> {
    sqlx::query_as(
        "SELECT installation.project_id,
                COALESCE(installation.organization_id, project.organization_id)
                    AS organization_id,
                installation.repository_id, installation.scope, installation.ui_key
         FROM ui_installations AS installation
         LEFT JOIN projects AS project ON project.id = installation.project_id
         WHERE installation.id = $1",
    )
    .bind(installation_id.as_uuid())
    .fetch_optional(&mut **tx)
    .await
}

async fn lock_installation(
    tx: &mut Transaction<'_, Postgres>,
    installation_id: UiInstallationId,
) -> Result<Option<LockedInstallationRow>, sqlx::Error> {
    sqlx::query_as(
        "SELECT installation.project_id,
                COALESCE(installation.organization_id, project.organization_id)
                    AS organization_id,
                installation.repository_id, installation.scope, installation.ui_key,
                installation.lifecycle, installation.current_generation_id,
                current_generation.generation_no AS current_generation_no
         FROM ui_installations AS installation
         JOIN ui_installation_generations AS current_generation
           ON current_generation.id = installation.current_generation_id
          AND current_generation.installation_id = installation.id
         LEFT JOIN projects AS project ON project.id = installation.project_id
         WHERE installation.id = $1
         FOR UPDATE OF installation",
    )
    .bind(installation_id.as_uuid())
    .fetch_optional(&mut **tx)
    .await
}

fn installation_target(
    owner: &InstallationOwnerRow,
) -> Option<release_domain::UiInstallationTarget> {
    match (owner.scope.as_str(), owner.project, owner.repository) {
        ("global", None, None) => Some(release_domain::UiInstallationTarget::organization(
            OrganizationId::from_uuid(owner.organization),
        )),
        ("project", Some(project_id), None) => Some(release_domain::UiInstallationTarget::project(
            ProjectId::from_uuid(project_id),
        )),
        ("repository", Some(_project_id), Some(repository_id)) => {
            Some(release_domain::UiInstallationTarget::repository(
                RepositoryId::from_uuid(repository_id),
            ))
        }
        _ => None,
    }
}

fn same_owner(initial: &InstallationOwnerRow, locked: &LockedInstallationRow) -> bool {
    initial.project == locked.project
        && initial.organization == locked.organization
        && initial.repository == locked.repository
        && initial.scope == locked.scope
        && initial.ui_key == locked.ui_key
}

async fn require_owner_management(
    service: &ReleaseService,
    tx: &mut Transaction<'_, Postgres>,
    identity: &AuthenticatedIdentity,
    target: release_domain::UiInstallationTarget,
    owner: &OwnerRow,
) -> Result<(), UiInstallationError> {
    match target {
        release_domain::UiInstallationTarget::Organization(_) => service
            .require(
                tx,
                identity,
                Permission::CanManage,
                ObjectRef::new(ObjectType::Organization, owner.organization),
            )
            .await
            .map_err(|error| map_authorization_public(&error)),
        release_domain::UiInstallationTarget::Project(_)
        | release_domain::UiInstallationTarget::Repository(_) => {
            let project_id = owner.project.ok_or(UiInstallationError::Unavailable)?;
            service
                .require(
                    tx,
                    identity,
                    Permission::CanManage,
                    ObjectRef::new(ObjectType::Project, project_id),
                )
                .await
                .map_err(|error| map_authorization_public(&error))?;
            if let Some(repository_id) = owner.repository {
                service
                    .require(
                        tx,
                        identity,
                        Permission::CanWrite,
                        ObjectRef::new(ObjectType::Repository, repository_id),
                    )
                    .await
                    .map_err(|error| map_authorization_public(&error))?;
            }
            Ok(())
        }
    }
}

fn lifecycle_state(value: &str) -> Option<UiInstallationState> {
    match value {
        "enabled" => Some(UiInstallationState::Enabled),
        "disabled" => Some(UiInstallationState::Disabled),
        "removed" => Some(UiInstallationState::Removed),
        _ => None,
    }
}

const fn lifecycle_name(value: UiInstallationState) -> &'static str {
    match value {
        UiInstallationState::Enabled => "enabled",
        UiInstallationState::Disabled => "disabled",
        UiInstallationState::Removed => "removed",
    }
}

fn replay_lifecycle_or_conflict(
    existing: &ExistingInstallCommand,
    command_key: ReleaseCommandKey,
    input_hash: UiInstallationInputDigest,
    actor_id: Uuid,
    installation_id: UiInstallationId,
    expected_state: UiInstallationState,
) -> Result<UiInstallationLifecycleResult, AttemptError> {
    if existing.command_key.as_slice() != command_key.as_bytes()
        || existing.input_hash.as_slice() != input_hash.as_bytes()
    {
        return Err(UiInstallationError::IdempotencyConflict.into());
    }
    if existing.installation_id != installation_id.as_uuid()
        || lifecycle_state(&existing.result_lifecycle) != Some(expected_state)
    {
        return Err(UiInstallationError::Unavailable.into());
    }
    Ok(UiInstallationLifecycleResult {
        installation_id,
        generation_id: UiInstallationGenerationId::from_uuid(existing.result_generation_id),
        state: expected_state,
        idempotency_id: actor_idempotency_id(actor_id.as_bytes(), command_key.as_bytes()).as_uuid(),
    })
}

async fn append_owner_event(
    tx: &mut Transaction<'_, Postgres>,
    occurrence_id: Uuid,
    owner: &OwnerRow,
) -> Result<(), sqlx::Error> {
    match (owner.project, owner.repository) {
        (None, None) => sqlx::query(
            "SELECT event_id
                 FROM append_application_event(
                     $1, 'organization', $2, 'organization', $2,
                     'organization.changed', 'updated', NULL, NULL, NULL
                 )",
        )
        .bind(occurrence_id)
        .bind(owner.organization)
        .fetch_one(&mut **tx)
        .await
        .map(|_| ()),
        (Some(project_id), None) => sqlx::query(
            "SELECT event_id
                 FROM append_application_event(
                     $1, 'project', $2, 'project', $2,
                     'project.changed', 'updated', NULL, $3, NULL
                 )",
        )
        .bind(occurrence_id)
        .bind(project_id)
        .bind(owner.organization)
        .fetch_one(&mut **tx)
        .await
        .map(|_| ()),
        (Some(project_id), Some(repository_id)) => sqlx::query(
            "SELECT event_id
                 FROM append_application_event(
                     $1, 'project', $2, 'repository', $3,
                     'repository.changed', 'updated', NULL, $2, NULL
                 )",
        )
        .bind(occurrence_id)
        .bind(project_id)
        .bind(repository_id)
        .fetch_one(&mut **tx)
        .await
        .map(|_| ()),
        (None, Some(_)) => Err(sqlx::Error::Protocol(
            "repository owner is missing its project".into(),
        )),
    }
}

async fn lock_owner(
    tx: &mut Transaction<'_, Postgres>,
    target: release_domain::UiInstallationTarget,
) -> Result<Option<OwnerRow>, sqlx::Error> {
    match target {
        release_domain::UiInstallationTarget::Project(project_id) => {
            sqlx::query_as(
                "SELECT project.id AS project_id, project.organization_id,
                        NULL::uuid AS repository_id
                 FROM projects AS project
                 WHERE project.id = $1
                 FOR NO KEY UPDATE OF project",
            )
            .bind(project_id.as_uuid())
            .fetch_optional(&mut **tx)
            .await
        }
        release_domain::UiInstallationTarget::Repository(repository_id) => {
            sqlx::query_as(
                "SELECT project.id AS project_id, project.organization_id,
                        repository.id AS repository_id
                 FROM repositories AS repository
                 JOIN projects AS project ON project.id = repository.project_id
                 WHERE repository.id = $1
                 FOR NO KEY UPDATE OF repository, project",
            )
            .bind(repository_id.as_uuid())
            .fetch_optional(&mut **tx)
            .await
        }
        release_domain::UiInstallationTarget::Organization(organization_id) => {
            sqlx::query_as(
                "SELECT NULL::uuid AS project_id, organization.id AS organization_id,
                        NULL::uuid AS repository_id
                 FROM organizations AS organization
                 WHERE organization.id = $1
                 FOR NO KEY UPDATE OF organization",
            )
            .bind(organization_id.as_uuid())
            .fetch_optional(&mut **tx)
            .await
        }
    }
}

async fn find_existing_command(
    tx: &mut Transaction<'_, Postgres>,
    actor_id: Uuid,
    operation: UiInstallationOperation,
    caller_key: &str,
) -> Result<Option<ExistingInstallCommand>, sqlx::Error> {
    sqlx::query_as(
        "SELECT command_key, input_hash, installation_id, result_generation_id,
                result_lifecycle
         FROM ui_installation_commands
         WHERE actor_id = $1 AND operation = $2
           AND caller_idempotency_key = $3",
    )
    .bind(actor_id)
    .bind(operation.as_str())
    .bind(caller_key)
    .fetch_optional(&mut **tx)
    .await
}

fn replay_or_conflict(
    existing: &ExistingInstallCommand,
    command_key: ReleaseCommandKey,
    input_hash: UiInstallationInputDigest,
    actor_id: Uuid,
) -> Result<InstallUiResult, AttemptError> {
    if existing.command_key.as_slice() != command_key.as_bytes()
        || existing.input_hash.as_slice() != input_hash.as_bytes()
    {
        return Err(UiInstallationError::IdempotencyConflict.into());
    }
    if existing.result_lifecycle != "enabled" {
        return Err(UiInstallationError::Unavailable.into());
    }
    Ok(InstallUiResult {
        installation_id: UiInstallationId::from_uuid(existing.installation_id),
        generation_id: UiInstallationGenerationId::from_uuid(existing.result_generation_id),
        state: UiInstallationState::Enabled,
        idempotency_id: actor_idempotency_id(actor_id.as_bytes(), command_key.as_bytes()).as_uuid(),
    })
}

fn replay_generation_or_conflict(
    existing: &ExistingInstallCommand,
    command_key: ReleaseCommandKey,
    input_hash: UiInstallationInputDigest,
    actor_id: Uuid,
    installation_id: UiInstallationId,
) -> Result<UiInstallationGenerationResult, AttemptError> {
    if existing.command_key.as_slice() != command_key.as_bytes()
        || existing.input_hash.as_slice() != input_hash.as_bytes()
    {
        return Err(UiInstallationError::IdempotencyConflict.into());
    }
    if existing.installation_id != installation_id.as_uuid()
        || existing.result_lifecycle != "enabled"
    {
        return Err(UiInstallationError::Unavailable.into());
    }
    Ok(UiInstallationGenerationResult {
        installation_id,
        generation_id: UiInstallationGenerationId::from_uuid(existing.result_generation_id),
        state: UiInstallationState::Enabled,
        idempotency_id: actor_idempotency_id(actor_id.as_bytes(), command_key.as_bytes()).as_uuid(),
    })
}

async fn insert_generation_bindings(
    tx: &mut Transaction<'_, Postgres>,
    installation_id: UiInstallationId,
    generation_id: UiInstallationGenerationId,
    release_id: ReleaseId,
    ui_key: &release_domain::ui::UiKey,
    bindings: &[ResolvedUiBinding],
) -> Result<(), UiInstallationError> {
    for binding in bindings {
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
        .bind(release_id.as_uuid())
        .bind(ui_key.as_str())
        .bind(binding.gateway_id)
        .bind(binding.gateway_revision_id)
        .bind(binding.release_agent_id)
        .bind(&binding.gateway_name)
        .bind(&binding.method)
        .bind(&binding.route)
        .execute(&mut **tx)
        .await
        .map_err(|_| UiInstallationError::Unavailable)?;
    }
    Ok(())
}

async fn active_installation(
    tx: &mut Transaction<'_, Postgres>,
    organization_id: Option<Uuid>,
    project_id: Option<Uuid>,
    repository_id: Option<Uuid>,
    ui_key: &release_domain::ui::UiKey,
) -> Result<bool, sqlx::Error> {
    Ok(sqlx::query_scalar::<_, Option<Uuid>>(
        "SELECT id
         FROM ui_installations
         WHERE organization_id IS NOT DISTINCT FROM $1
           AND project_id IS NOT DISTINCT FROM $2
           AND repository_id IS NOT DISTINCT FROM $3
           AND ui_key = $4 AND lifecycle <> 'removed'
         FOR UPDATE",
    )
    .bind(organization_id)
    .bind(project_id)
    .bind(repository_id)
    .bind(ui_key.as_str())
    .fetch_optional(&mut **tx)
    .await?
    .is_some())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UiBindingResolutionError {
    Invalid,
    Persistence,
}

#[derive(Debug, sqlx::FromRow)]
struct GatewayRouteCandidate {
    gateway_id: Uuid,
    gateway_revision_id: Uuid,
    gateway_name: String,
    handler_contract: String,
    route_path: String,
    route_methods: Vec<String>,
}

#[derive(Debug, Clone)]
struct ResolvedUiBinding {
    binding_kind: &'static str,
    binding_key: String,
    gateway_id: Uuid,
    gateway_revision_id: Uuid,
    release_agent_id: Uuid,
    gateway_name: String,
    method: String,
    route: String,
}

// Keep target-shape, gateway, and binding validation together so every
// installation scope follows the same publication and authorization path.
#[allow(clippy::too_many_lines)]
async fn resolve_ui_bindings(
    tx: &mut Transaction<'_, Postgres>,
    actor_id: Uuid,
    release_id: ReleaseId,
    target_organization_id: Uuid,
    target_scope: &str,
    ui_key: &release_domain::ui::UiKey,
    static_zero_api_only: bool,
) -> Result<Vec<ResolvedUiBinding>, UiBindingResolutionError> {
    let publication: Option<(String, String, Uuid, Uuid)> = sqlx::query_as(
        "SELECT descriptor.scope, descriptor.content_kind,
                source_repository.id, source_project.id
         FROM release_ui_descriptors AS descriptor
         JOIN releases AS release ON release.id = descriptor.release_id
         JOIN repositories AS source_repository
           ON source_repository.id = release.repository_id
         JOIN projects AS source_project
           ON source_project.id = source_repository.project_id
         WHERE descriptor.release_id = $1
           AND descriptor.ui_key = $2
           AND descriptor.scope = $3
           AND source_project.organization_id = $4
           AND release.state = 'published'
         FOR SHARE OF release, source_repository, source_project",
    )
    .bind(release_id.as_uuid())
    .bind(ui_key.as_str())
    .bind(target_scope)
    .bind(target_organization_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|_| UiBindingResolutionError::Persistence)?;
    let Some((scope, content_kind, source_repository_id, source_project_id)) = publication else {
        return Err(UiBindingResolutionError::Invalid);
    };
    if scope != target_scope {
        return Err(UiBindingResolutionError::Invalid);
    }

    let managed: Option<(String, String, Uuid)> = sqlx::query_as(
        "SELECT gateway_name, route, release_agent_id
         FROM release_ui_managed_services
         WHERE release_id = $1 AND ui_key = $2",
    )
    .bind(release_id.as_uuid())
    .bind(ui_key.as_str())
    .fetch_optional(&mut **tx)
    .await
    .map_err(|_| UiBindingResolutionError::Persistence)?;
    let apis: Vec<(String, String, String, String, Uuid)> = sqlx::query_as(
        "SELECT api_key, gateway_name, method, route, release_agent_id
         FROM release_ui_api_bindings
         WHERE release_id = $1 AND ui_key = $2
         ORDER BY api_key",
    )
    .bind(release_id.as_uuid())
    .bind(ui_key.as_str())
    .fetch_all(&mut **tx)
    .await
    .map_err(|_| UiBindingResolutionError::Persistence)?;

    if static_zero_api_only && (content_kind != "static" || managed.is_some() || !apis.is_empty()) {
        return Err(UiBindingResolutionError::Invalid);
    }
    if content_kind == "static" {
        let has_file: bool = sqlx::query_scalar(
            "SELECT EXISTS (
                 SELECT 1 FROM release_ui_static_files
                 WHERE release_id = $1 AND ui_key = $2
             )",
        )
        .bind(release_id.as_uuid())
        .bind(ui_key.as_str())
        .fetch_one(&mut **tx)
        .await
        .map_err(|_| UiBindingResolutionError::Persistence)?;
        if !has_file || managed.is_some() {
            return Err(UiBindingResolutionError::Invalid);
        }
    } else if content_kind == "managed_service" {
        if managed.is_none() {
            return Err(UiBindingResolutionError::Invalid);
        }
    } else {
        return Err(UiBindingResolutionError::Invalid);
    }

    let mut bindings = Vec::with_capacity(usize::from(managed.is_some()) + apis.len());
    if let Some((gateway_name, route, release_agent_id)) = managed {
        let candidate = resolve_gateway_route(
            tx,
            actor_id,
            source_repository_id,
            source_project_id,
            release_id,
            &gateway_name,
            release_agent_id,
            "GET",
            &route,
            true,
        )
        .await?;
        bindings.push(ResolvedUiBinding {
            binding_kind: "managed_service",
            binding_key: String::from("service"),
            gateway_id: candidate.gateway_id,
            gateway_revision_id: candidate.gateway_revision_id,
            release_agent_id,
            gateway_name: candidate.gateway_name,
            method: String::from("GET"),
            route,
        });
    }
    for (binding_key, gateway_name, method, route, release_agent_id) in apis {
        let candidate = resolve_gateway_route(
            tx,
            actor_id,
            source_repository_id,
            source_project_id,
            release_id,
            &gateway_name,
            release_agent_id,
            &method,
            &route,
            false,
        )
        .await?;
        bindings.push(ResolvedUiBinding {
            binding_kind: "api",
            binding_key,
            gateway_id: candidate.gateway_id,
            gateway_revision_id: candidate.gateway_revision_id,
            release_agent_id,
            gateway_name: candidate.gateway_name,
            method,
            route,
        });
    }
    Ok(bindings)
}

#[allow(clippy::too_many_arguments)]
async fn resolve_gateway_route(
    tx: &mut Transaction<'_, Postgres>,
    actor_id: Uuid,
    source_repository_id: Uuid,
    source_project_id: Uuid,
    release_id: ReleaseId,
    gateway_name: &str,
    release_agent_id: Uuid,
    method: &str,
    selected_route: &str,
    managed_service: bool,
) -> Result<GatewayRouteCandidate, UiBindingResolutionError> {
    let candidates: Vec<GatewayRouteCandidate> = sqlx::query_as(
        "SELECT gateway.id AS gateway_id, revision.id AS gateway_revision_id,
                gateway.name AS gateway_name,
                revision.handler_contract, route.path AS route_path,
                route.methods AS route_methods
         FROM gateways AS gateway
         JOIN gateway_revisions AS revision
           ON revision.gateway_id = gateway.id
          AND revision.id = gateway.active_revision_id
          AND revision.release_id = $3
          AND revision.release_agent_id = $4
          AND revision.exposure = 'heph_authenticated'
         JOIN gateway_routes AS route
           ON route.gateway_id = gateway.id
          AND route.gateway_revision_id = revision.id
          AND route.enabled
         WHERE gateway.repository_id = $1
           AND gateway.project_id = $2
           AND gateway.name = $5
           AND gateway.lifecycle = 'enabled'
           AND check_permission(
                 'user', $6, 'can_read', 'project', gateway.project_id::text
               ) = 1
         FOR UPDATE OF gateway",
    )
    .bind(source_repository_id)
    .bind(source_project_id)
    .bind(release_id.as_uuid())
    .bind(release_agent_id)
    .bind(gateway_name)
    .bind(actor_id.to_string())
    .fetch_all(&mut **tx)
    .await
    .map_err(|_| UiBindingResolutionError::Persistence)?;
    candidates
        .into_iter()
        .find(|candidate| {
            (!managed_service || candidate.handler_contract == "http.service.v1")
                && (managed_service
                    || matches!(
                        candidate.handler_contract.as_str(),
                        "http.v1" | "http.service.v1"
                    ))
                && candidate.route_methods.iter().any(|value| value == method)
                && route_covers(&candidate.route_path, selected_route)
        })
        .ok_or(UiBindingResolutionError::Invalid)
}

fn route_covers(declared: &str, selected: &str) -> bool {
    selected == declared
        || selected
            .strip_prefix(declared)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

// Keep the owner columns explicit so each target shape is visible at the SQL
// boundary; grouping them would hide the global/project/repository invariant.
#[allow(clippy::too_many_arguments)]
async fn insert_installation(
    tx: &mut Transaction<'_, Postgres>,
    installation_id: UiInstallationId,
    generation_id: UiInstallationGenerationId,
    organization_id: Option<Uuid>,
    project_id: Option<Uuid>,
    repository_id: Option<Uuid>,
    scope: &str,
    ui_key: &release_domain::ui::UiKey,
    actor_id: Uuid,
) -> Result<(), AttemptError> {
    sqlx::query(
        "INSERT INTO ui_installations
         (id, organization_id, project_id, repository_id, scope, ui_key, lifecycle,
          current_generation_id, created_by)
         VALUES ($1, $2, $3, $4, $5, $6, 'enabled', $7, $8)",
    )
    .bind(installation_id.as_uuid())
    .bind(organization_id)
    .bind(project_id)
    .bind(repository_id)
    .bind(scope)
    .bind(ui_key.as_str())
    .bind(generation_id.as_uuid())
    .bind(actor_id)
    .execute(&mut **tx)
    .await
    .map(|_| ())
    .map_err(|error| map_insert_error(&error))
}

fn map_authorization_error(error: &super::ReleaseServiceError) -> AttemptError {
    match error {
        super::ReleaseServiceError::AuthorizationDenied => {
            UiInstallationError::PermissionDenied.into()
        }
        super::ReleaseServiceError::Database(_) | super::ReleaseServiceError::Authorization(_) => {
            UiInstallationError::Unavailable.into()
        }
        _ => UiInstallationError::Unavailable.into(),
    }
}

fn map_authorization_public(error: &super::ReleaseServiceError) -> UiInstallationError {
    match map_authorization_error(error) {
        AttemptError::Public(error) => error,
        AttemptError::RetryLedgerRace => UiInstallationError::Unavailable,
    }
}

fn map_insert_error(error: &sqlx::Error) -> AttemptError {
    if let Some(database_error) = error.as_database_error() {
        if database_error.code().as_deref() == Some("23505")
            && database_error
                .constraint()
                .is_some_and(|name| name == "ui_installations_active_owner_key")
        {
            return UiInstallationError::AlreadyInstalled.into();
        }
    }
    UiInstallationError::Unavailable.into()
}

fn map_command_insert_error(error: &sqlx::Error) -> AttemptError {
    if let Some(database_error) = error.as_database_error() {
        // This INSERT has only the command-key primary key and the
        // actor/operation/caller-key uniqueness constraint. PostgreSQL may
        // truncate the generated latter name, so match the SQLSTATE rather
        // than guessing an identifier.
        if database_error.code().as_deref() == Some("23505") {
            return AttemptError::RetryLedgerRace;
        }
    }
    UiInstallationError::Unavailable.into()
}
