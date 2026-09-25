use super::AttemptError;
use release_domain::{UiInstallationGenerationId, UiInstallationId};
use release_service::UiInstallationError;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

// Keep the owner columns explicit so each target shape is visible at the SQL
// boundary; grouping them would hide the global/project/repository invariant.
#[allow(clippy::too_many_arguments)]
pub(super) async fn insert_installation(
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

pub(super) fn map_authorization_error(error: &crate::ReleaseServiceError) -> AttemptError {
    match error {
        crate::ReleaseServiceError::AuthorizationDenied => {
            UiInstallationError::PermissionDenied.into()
        }
        crate::ReleaseServiceError::Database(_) | crate::ReleaseServiceError::Authorization(_) => {
            UiInstallationError::Unavailable.into()
        }
        _ => UiInstallationError::Unavailable.into(),
    }
}

pub(super) fn map_authorization_public(error: &crate::ReleaseServiceError) -> UiInstallationError {
    match map_authorization_error(error) {
        AttemptError::Public(error) => error,
        AttemptError::RetryLedgerRace => UiInstallationError::Unavailable,
    }
}

pub(super) fn map_insert_error(error: &sqlx::Error) -> AttemptError {
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

pub(super) fn map_command_insert_error(error: &sqlx::Error) -> AttemptError {
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
