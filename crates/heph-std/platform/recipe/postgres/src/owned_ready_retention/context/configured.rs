use super::{Context, PostgresOwnedReadyRetentionRepository};
use crate::{execution_preparation, remove_resume, repository_error};
use recipe_application::{CommandIdentity, DeploymentError, RemoveResumeConfiguration};
use release_domain::ContentHash;

/// Returns only the immutable FIRST input after complete configured replay
/// qualification. The namespace/host are retained adapter pins; the restored
/// platform/profile have already passed its trusted preparation validator.
pub async fn original_input(
    adapter: &PostgresOwnedReadyRetentionRepository,
    context: &mut Context<'_>,
    command: CommandIdentity,
) -> Result<Option<ContentHash>, DeploymentError> {
    let recorded: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM recipe_remove_resume_admissions WHERE command_id=$1)",
    )
    .bind(command.id().as_uuid())
    .fetch_one(&mut *context.tx)
    .await
    .map_err(repository_error)?;
    if !recorded {
        return Ok(None);
    }
    adapter
        .preparation
        .assert_ready_retention_configuration(&mut context.tx, &context.snapshot.intent)
        .await?;
    let prepared =
        execution_preparation::hydration::load(&mut context.tx, &context.snapshot.intent)
            .await?
            .ok_or(DeploymentError::ReconciliationRequired)?
            .preparation;
    let config = RemoveResumeConfiguration::new(
        adapter.namespace.clone(),
        adapter.host.clone(),
        prepared.platform().clone(),
        prepared.profile(),
    )?;
    let input = remove_resume::context::current_input(
        &mut context.tx,
        command,
        &context.snapshot,
        Some(&config),
    )
    .await?;
    input
        .ok_or(DeploymentError::ReconciliationRequired)
        .map(Some)
}

pub fn receipt(
    lifecycle: &str,
    input: &[u8],
    configured: Option<ContentHash>,
) -> Result<(), DeploymentError> {
    if let Some(original) = configured {
        if !matches!(lifecycle, "removing" | "recovery_required") || input != original.as_bytes() {
            return Err(DeploymentError::InputConflict);
        }
    } else if lifecycle != "removing" {
        return Err(DeploymentError::InputConflict);
    }
    Ok(())
}
