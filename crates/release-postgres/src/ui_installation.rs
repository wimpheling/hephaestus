//! `PostgreSQL` adapter for the first static, zero-API UI installation command.

use authz_domain::{ObjectRef, ObjectType, Permission};
use forge_domain::ProjectId;
use identity_domain::{AuthenticatedIdentity, actor_idempotency_id};
use release_domain::{
    ReleaseCommandKey, ReleaseId, UiInstallationCommandIdentity, UiInstallationGenerationId,
    UiInstallationId, UiInstallationInputDigest, UiInstallationState,
};
use release_service::{InstallStaticUi, InstallStaticUiResult, UiInstallationError};
use sqlx::{FromRow, Postgres, Transaction};
use uuid::Uuid;

use super::ReleaseService;

#[derive(Debug, FromRow)]
struct OwnerRow {
    #[sqlx(rename = "project_id")]
    project: Uuid,
    #[sqlx(rename = "organization_id")]
    organization: Uuid,
    #[sqlx(rename = "repository_id")]
    repository: Option<Uuid>,
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

    // Keep the ordered transaction in one function so its rollback and
    // bounded idempotency retry boundary remain directly auditable.
    #[allow(clippy::too_many_lines)]
    async fn install_static_ui_once(
        &self,
        identity: &AuthenticatedIdentity,
        command: &InstallStaticUi,
    ) -> Result<InstallStaticUiResult, AttemptError> {
        if matches!(
            command.target,
            release_domain::UiInstallationTarget::Organization(_)
        ) {
            return Err(UiInstallationError::InvalidOrUnsupported.into());
        }
        let mut tx = authz_postgres::begin_actor_transaction(&self.pool, identity)
            .await
            .map_err(|_| UiInstallationError::Unavailable)?;
        let owner = lock_owner(&mut tx, command.target)
            .await
            .map_err(|_| UiInstallationError::Unavailable)?
            .ok_or(UiInstallationError::Unavailable)?;
        self.require(
            &mut tx,
            identity,
            Permission::CanManage,
            ObjectRef::new(ObjectType::Project, owner.project),
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
            owner.repository,
            &command.ui_key,
        )
        .await
        .map_err(|_| UiInstallationError::Unavailable)?;
        if !supported {
            return Err(UiInstallationError::InvalidOrUnsupported.into());
        }
        let already_active =
            active_installation(&mut tx, owner.project, owner.repository, &command.ui_key)
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
            ProjectId::from_uuid(owner.project),
            owner.repository,
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

async fn append_owner_event(
    tx: &mut Transaction<'_, Postgres>,
    occurrence_id: Uuid,
    owner: &OwnerRow,
) -> Result<(), sqlx::Error> {
    match owner.repository {
        None => sqlx::query(
            "SELECT event_id
                 FROM append_application_event(
                     $1, 'project', $2, 'project', $2,
                     'project.changed', 'updated', NULL, $3, NULL
                 )",
        )
        .bind(occurrence_id)
        .bind(owner.project)
        .bind(owner.organization)
        .fetch_one(&mut **tx)
        .await
        .map(|_| ()),
        Some(repository_id) => sqlx::query(
            "SELECT event_id
                 FROM append_application_event(
                     $1, 'project', $2, 'repository', $3,
                     'repository.changed', 'updated', NULL, $2, NULL
                 )",
        )
        .bind(occurrence_id)
        .bind(owner.project)
        .bind(repository_id)
        .fetch_one(&mut **tx)
        .await
        .map(|_| ()),
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
        release_domain::UiInstallationTarget::Organization(_) => Ok(None),
    }
}

async fn find_existing_command(
    tx: &mut Transaction<'_, Postgres>,
    actor_id: Uuid,
    caller_key: &str,
) -> Result<Option<ExistingInstallCommand>, sqlx::Error> {
    sqlx::query_as(
        "SELECT command_key, input_hash, installation_id, result_generation_id,
                result_lifecycle
         FROM ui_installation_commands
         WHERE actor_id = $1 AND operation = 'install'
           AND caller_idempotency_key = $2",
    )
    .bind(actor_id)
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
    project_id: Uuid,
    repository_id: Option<Uuid>,
    ui_key: &release_domain::ui::UiKey,
) -> Result<bool, sqlx::Error> {
    Ok(sqlx::query_scalar::<_, Option<Uuid>>(
        "SELECT id
         FROM ui_installations
         WHERE project_id = $1 AND repository_id IS NOT DISTINCT FROM $2
           AND ui_key = $3 AND lifecycle <> 'removed'
         FOR UPDATE",
    )
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
    repository_id: Option<Uuid>,
    ui_key: &release_domain::ui::UiKey,
) -> Result<bool, sqlx::Error> {
    let scope = if repository_id.is_some() {
        "repository"
    } else {
        "project"
    };
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
    .bind(scope)
    .bind(target_organization_id)
    .fetch_optional(&mut **tx)
    .await?;
    Ok(valid.is_some())
}

async fn insert_installation(
    tx: &mut Transaction<'_, Postgres>,
    installation_id: UiInstallationId,
    generation_id: UiInstallationGenerationId,
    project_id: ProjectId,
    repository_id: Option<Uuid>,
    ui_key: &release_domain::ui::UiKey,
    actor_id: Uuid,
) -> Result<(), AttemptError> {
    sqlx::query(
        "INSERT INTO ui_installations
         (id, project_id, repository_id, scope, ui_key, lifecycle,
          current_generation_id, created_by)
         VALUES ($1, $2, $3, $4, $5, 'enabled', $6, $7)",
    )
    .bind(installation_id.as_uuid())
    .bind(project_id.as_uuid())
    .bind(repository_id)
    .bind(if repository_id.is_some() {
        "repository"
    } else {
        "project"
    })
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
