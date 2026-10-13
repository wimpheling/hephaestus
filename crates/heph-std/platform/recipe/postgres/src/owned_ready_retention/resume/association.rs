use crate::{effect_context::Context, repository_error};
use identity_domain::AuthenticatedIdentity;
use recipe_application::{CommandReceipt, DeploymentError, DeploymentLifecycle, RemoveDeployment};
use release_domain::ContentHash;
use uuid::Uuid;

#[cfg(test)]
#[path = "association/tests.rs"]
mod tests;

pub async fn ensure(
    current: &mut Context<'_>,
    identity: &AuthenticatedIdentity,
    remove: &RemoveDeployment,
    closure: Uuid,
    receipt: &CommandReceipt,
    configured: Option<ContentHash>,
) -> Result<(), DeploymentError> {
    let expected = version(
        current.snapshot.lifecycle,
        current.snapshot.version,
        remove.expected_version,
        receipt.input_hash,
        configured,
    )?;
    let saved: Option<(i64, bool)> = sqlx::query_as(
        "SELECT expected_version,(actor_id=$3 AND event_id=$4) FROM recipe_owned_ready_remove_admissions
         WHERE command_id=$1 AND closure_id=$2",
    )
    .bind(remove.command.id().as_uuid())
    .bind(closure)
    .bind(remove.command.actor_id().as_uuid())
    .bind(receipt.event.event_id)
    .fetch_optional(&mut *current.tx)
    .await
    .map_err(repository_error)?;
    if let Some((saved, exact)) = saved {
        if !exact || saved != expected {
            return Err(DeploymentError::InputConflict);
        }
        return Ok(());
    }
    if configured.is_none() {
        return Err(DeploymentError::InputConflict);
    }
    // The current request authorizes only this association. The command event,
    // input hash, first request and receipt remain immutable original data.
    sqlx::query("INSERT INTO recipe_owned_ready_remove_admissions(command_id,closure_id,expected_version,actor_id,request_id,event_id) VALUES($1,$2,$3,$4,$5,$6)")
        .bind(remove.command.id().as_uuid()).bind(closure).bind(expected)
        .bind(identity.user_id.as_uuid()).bind(identity.request_id.as_uuid()).bind(receipt.event.event_id)
        .execute(&mut *current.tx).await.map_err(repository_error)?;
    Ok(())
}

fn version(
    lifecycle: DeploymentLifecycle,
    current: u64,
    original: u64,
    input: ContentHash,
    configured: Option<ContentHash>,
) -> Result<i64, DeploymentError> {
    if let Some(recorded) = configured {
        if recorded != input
            || !matches!(
                lifecycle,
                DeploymentLifecycle::Removing | DeploymentLifecycle::RecoveryRequired
            )
            || current == 0
        {
            return Err(DeploymentError::InputConflict);
        }
        i64::try_from(current).map_err(|_| DeploymentError::StaleClaim)
    } else {
        i64::try_from(original).map_err(|_| DeploymentError::StaleClaim)
    }
}
