//! Current resource must be the actual Applied birth or its distinct Ready retention.
use super::ledger;
use crate::{effect_state::State, repository_error};
use recipe_application::{
    DeploymentError, DeploymentSnapshot, EffectClaim, InstallProgress, RemovalProgress,
    RemoveResumeConfiguration,
};
use sqlx::{Postgres, Transaction};

mod retained;
#[cfg(test)]
#[path = "current/tests.rs"]
mod tests;

pub(super) async fn qualify(
    tx: &mut Transaction<'_, Postgres>,
    snapshot: &DeploymentSnapshot,
    claim: &EffectClaim,
    ready: &State,
    config: &RemoveResumeConfiguration,
) -> Result<(), DeploymentError> {
    let state: serde_json::Value = sqlx::query_scalar(
        "SELECT recipe_effect_resource_state(resource) FROM recipe_deployment_resources resource
         WHERE deployment_id=$1 AND resource_name=$2",
    )
    .bind(claim.deployment_id.as_uuid())
    .bind(claim.resource.as_str())
    .fetch_one(&mut **tx)
    .await
    .map_err(repository_error)?;
    let current = ledger::decode(&state)?;
    match current.removal {
        RemovalProgress::Pending if current == *ready => Ok(()),
        RemovalProgress::Retained if current.install == InstallProgress::Ready => {
            retained::qualify(tx, snapshot, claim, ready, &current, config).await
        }
        _ => Err(DeploymentError::ReconciliationRequired),
    }
}
