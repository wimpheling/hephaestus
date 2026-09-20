//! `PostgreSQL` adapter for static UI installation and lifecycle commands.

use authz_domain::{ObjectRef, ObjectType, Permission};
use forge_domain::{OrganizationId, ProjectId, RepositoryId};
use identity_domain::{AuthenticatedIdentity, actor_idempotency_id};
use release_domain::{
    ReleaseCommandKey, ReleaseId, UiInstallationCommandIdentity, UiInstallationGenerationId,
    UiInstallationId, UiInstallationInputDigest, UiInstallationOperation, UiInstallationState,
};
use release_service::{
    DisableUiInstallation, InstallStaticUi, InstallStaticUiResult, RemoveUiInstallation,
    UiInstallationError, UiInstallationLifecycleResult,
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
    lifecycle: String,
    current_generation_id: Uuid,
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
    /// Installs one published static UI with no API bindings.
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
        for attempt in 0..2 {
            match self.install_static_ui_once(identity, &command).await {
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
    #[allow(clippy::too_many_lines)]
    async fn install_static_ui_once(
        &self,
        identity: &AuthenticatedIdentity,
        command: &InstallStaticUi,
    ) -> Result<InstallStaticUiResult, AttemptError> {
        let mut tx = authz_postgres::begin_actor_transaction(&self.pool, identity)
            .await
            .map_err(|_| UiInstallationError::Unavailable)?;
        let owner = lock_owner(&mut tx, command.target)
            .await
            .map_err(|_| UiInstallationError::Unavailable)?
            .ok_or(UiInstallationError::Unavailable)?;
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
        let input_hash =
            UiInstallationInputDigest::install(command.target, command.release_id, &command.ui_key);
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

        self.require(
            &mut tx,
            identity,
            Permission::CanUse,
            ObjectRef::new(ObjectType::Release, command.release_id.as_uuid()),
        )
        .await
        .map_err(|error| map_authorization_error(&error))?;
        let supported = ensure_static_zero_api_ui(
            &mut tx,
            command.release_id,
            owner.organization,
            command.target.scope_name(),
            &command.ui_key,
        )
        .await
        .map_err(|_| UiInstallationError::Unavailable)?;
        if !supported {
            return Err(UiInstallationError::InvalidOrUnsupported.into());
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
        Ok(InstallStaticUiResult {
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
                installation.repository_id, installation.scope
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
                installation.repository_id, installation.scope, installation.lifecycle,
                installation.current_generation_id
         FROM ui_installations AS installation
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
) -> Result<InstallStaticUiResult, AttemptError> {
    if existing.command_key.as_slice() != command_key.as_bytes()
        || existing.input_hash.as_slice() != input_hash.as_bytes()
    {
        return Err(UiInstallationError::IdempotencyConflict.into());
    }
    if existing.result_lifecycle != "enabled" {
        return Err(UiInstallationError::Unavailable.into());
    }
    Ok(InstallStaticUiResult {
        installation_id: UiInstallationId::from_uuid(existing.installation_id),
        generation_id: UiInstallationGenerationId::from_uuid(existing.result_generation_id),
        state: UiInstallationState::Enabled,
        idempotency_id: actor_idempotency_id(actor_id.as_bytes(), command_key.as_bytes()).as_uuid(),
    })
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

async fn ensure_static_zero_api_ui(
    tx: &mut Transaction<'_, Postgres>,
    release_id: ReleaseId,
    target_organization_id: Uuid,
    target_scope: &str,
    ui_key: &release_domain::ui::UiKey,
) -> Result<bool, sqlx::Error> {
    let valid: Option<i64> = sqlx::query_scalar(
        "SELECT 1::bigint
         FROM release_ui_descriptors AS descriptor
         JOIN releases AS release ON release.id = descriptor.release_id
         JOIN repositories AS source_repository
           ON source_repository.id = release.repository_id
         JOIN projects AS source_project
           ON source_project.id = source_repository.project_id
         WHERE descriptor.release_id = $1 AND descriptor.ui_key = $2
           AND descriptor.scope = $3 AND descriptor.content_kind = 'static'
           AND source_project.organization_id = $4
           AND release.state = 'published'
           AND EXISTS (
               SELECT 1 FROM release_ui_static_files AS file
               WHERE file.release_id = descriptor.release_id
                 AND file.ui_key = descriptor.ui_key
           )
           AND NOT EXISTS (
               SELECT 1 FROM release_ui_api_bindings AS api
               WHERE api.release_id = descriptor.release_id
                 AND api.ui_key = descriptor.ui_key
           )
           AND NOT EXISTS (
               SELECT 1 FROM release_ui_managed_services AS service
               WHERE service.release_id = descriptor.release_id
                 AND service.ui_key = descriptor.ui_key
           )
           ",
    )
    .bind(release_id.as_uuid())
    .bind(ui_key.as_str())
    .bind(target_scope)
    .bind(target_organization_id)
    .fetch_optional(&mut **tx)
    .await?;
    Ok(valid.is_some())
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
