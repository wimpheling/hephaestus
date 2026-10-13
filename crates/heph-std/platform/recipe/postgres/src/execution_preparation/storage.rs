use authz_postgres::AUTHORIZATION_MODEL_VERSION;
use event_application::CommittedMutation;
use recipe_application::{DeploymentError, DeploymentExecutionPreparation};
use sqlx::{Postgres, Transaction};

use crate::repository_error;

pub struct Written {
    pub event: CommittedMutation,
    pub replayed: bool,
}

pub async fn write(
    tx: &mut Transaction<'_, Postgres>,
    preparation: &DeploymentExecutionPreparation,
) -> Result<Written, DeploymentError> {
    let children: Vec<Vec<u8>> = preparation
        .instances()
        .iter()
        .map(serde_json::to_vec)
        .collect::<Result<_, _>>()
        .map_err(|_| DeploymentError::Serialization)?;
    let row: super::rows::WriterRow = sqlx::query_as(
        "SELECT event_id,event_cursor,event_aggregate_version,replayed
         FROM write_recipe_execution_preparation($1,$2,$3,$4,$5)",
    )
    .bind(preparation.deployment_id().as_uuid())
    .bind(preparation.command().id().as_uuid())
    .bind(preparation.canonical_bytes()?)
    .bind(children)
    .bind(AUTHORIZATION_MODEL_VERSION)
    .fetch_one(&mut **tx)
    .await
    .map_err(repository_error)?;
    if row.event_id.is_nil() || row.event_cursor <= 0 || row.event_aggregate_version <= 0 {
        return Err(DeploymentError::IntentMismatch);
    }
    Ok(Written {
        event: CommittedMutation {
            event_id: row.event_id,
            scope_kind: String::from("project"),
            scope_id: preparation.project_id().as_uuid(),
            cursor: row.event_cursor,
            aggregate_version: row.event_aggregate_version,
        },
        replayed: row.replayed,
    })
}
