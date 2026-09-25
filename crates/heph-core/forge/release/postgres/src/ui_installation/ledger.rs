use super::owner::{lifecycle_state, receipt_scope};
use super::{AttemptError, ExistingInstallCommand, OwnerRow, ResolvedUiBinding};
use identity_domain::actor_idempotency_id;
use release_domain::{
    ReleaseCommandKey, ReleaseId, UiInstallationGenerationId, UiInstallationId,
    UiInstallationInputDigest, UiInstallationOperation, UiInstallationState, UiInstallationTarget,
};
use release_service::{
    InstallUiResult, UiInstallationError, UiInstallationGenerationResult,
    UiInstallationLifecycleResult,
};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

pub(super) fn replay_lifecycle_or_conflict(
    existing: &ExistingInstallCommand,
    command_key: ReleaseCommandKey,
    input_hash: UiInstallationInputDigest,
    actor_id: Uuid,
    installation_id: UiInstallationId,
    expected_state: UiInstallationState,
    target: UiInstallationTarget,
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
        receipt_scope: receipt_scope(target),
        generation_id: UiInstallationGenerationId::from_uuid(existing.result_generation_id),
        state: expected_state,
        idempotency_id: actor_idempotency_id(actor_id.as_bytes(), command_key.as_bytes()).as_uuid(),
    })
}

pub(super) async fn append_owner_event(
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

pub(super) async fn lock_owner(
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

pub(super) async fn find_existing_command(
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

pub(super) fn replay_or_conflict(
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

pub(super) fn replay_generation_or_conflict(
    existing: &ExistingInstallCommand,
    command_key: ReleaseCommandKey,
    input_hash: UiInstallationInputDigest,
    actor_id: Uuid,
    installation_id: UiInstallationId,
    target: UiInstallationTarget,
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
        receipt_scope: receipt_scope(target),
        generation_id: UiInstallationGenerationId::from_uuid(existing.result_generation_id),
        state: UiInstallationState::Enabled,
        idempotency_id: actor_idempotency_id(actor_id.as_bytes(), command_key.as_bytes()).as_uuid(),
    })
}

pub(super) async fn insert_generation_bindings(
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

pub(super) async fn active_installation(
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
