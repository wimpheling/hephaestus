//! Authorized observation of one immutable terminal result, without admitting work.
mod authority;
mod resources;
mod rows;
#[cfg(test)]
mod tests;

use crate::{PostgresDeploymentRepository, hydration, remove_resume::context, repository_error};
use identity_domain::AuthenticatedIdentity;
use recipe_application::{CommandReceipt, DeploymentError, DeploymentIntent};

pub(super) async fn load(
    repository: &PostgresDeploymentRepository,
    identity: &AuthenticatedIdentity,
    intent: &DeploymentIntent,
) -> Result<Option<(CommandReceipt, u64)>, DeploymentError> {
    let config = repository
        .remove_resume
        .as_ref()
        .ok_or(DeploymentError::InvalidAction)?;
    let mut tx = repository
        .pool
        .begin_with("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .await
        .map_err(repository_error)?;
    let result = async {
        authority::actor(&mut tx, identity).await?;
        authority::project(&mut tx, identity, intent).await?;
        let snapshot = hydration::load(&mut tx, intent.id()).await?;
        if snapshot.intent != *intent {
            return Err(DeploymentError::IntentMismatch);
        }
        authority::owned(&mut tx, identity, intent).await?;
        context::configured(&mut tx, intent, config).await?;
        let completed = rows::load(&mut tx, &snapshot).await?;
        let Some(row) = rows::select(&completed, &snapshot)? else {
            return Ok(None);
        };
        let completed = rows::restore(row, &snapshot)?;
        context::command_configuration(&mut tx, completed.0.command, config).await?;
        resources::qualify(&mut tx, &snapshot, config).await?;
        Ok(Some(completed))
    }
    .await;
    let rollback = tx.rollback().await.map_err(repository_error);
    match result {
        Err(primary) => Err(primary),
        Ok(value) => rollback.map(|()| value),
    }
}
