use run_domain::{Run, RunKind, StartRun};
use run_orchestrator::{RunRuntimeCatalogError, RunRuntimeInput};

use super::utility::storage;
use crate::{PgRunRepository, model::RunRow};

impl PgRunRepository {
    pub(crate) async fn load_invocation_runtime(
        &self,
        run: &Run,
    ) -> Result<RunRuntimeInput, RunRuntimeCatalogError> {
        let configuration = self.invocation_configuration().map_err(|_| {
            RunRuntimeCatalogError::InvalidData(
                "qualified Invocation configuration is absent or differs",
            )
        })?;
        let command = StartRun {
            command_id: run.command_id,
            run_id: run.id,
            instance_id: run.instance_id,
            instance_revision_id: run.instance_revision_id,
            release_id: run.release_id,
            release_agent_id: run.release_agent_id,
            attachment_id: run.attachment_id,
            kind: RunKind::Invocation,
            requires_state: run.requires_state,
        };
        let payload = serde_json::to_value(&command)
            .map_err(|_| RunRuntimeCatalogError::InvalidData("Invocation Start serialization"))?;
        let config = serde_json::to_vec(&configuration.canonical_json()).map_err(|_| {
            RunRuntimeCatalogError::InvalidData("Invocation configuration serialization")
        })?;
        let mut tx = self
            .profile_inventory_transaction()
            .await
            .map_err(|error| RunRuntimeCatalogError::Storage(Box::new(error)))?;
        let reservation = if self.invocation_lease_runtime {
            crate::invocation_admission::reservations119::acquire(&mut tx, run.id).await
        } else {
            crate::invocation_admission::reservations::acquire(&mut tx, run.id).await
        };
        reservation.map_err(|error| RunRuntimeCatalogError::Storage(Box::new(error)))?;
        let stored =
            read_runtime(&mut tx, run, payload, config, self.invocation_lease_runtime).await?;
        let checked = stored
            .into_checked_invocation()
            .map_err(|_| RunRuntimeCatalogError::InvalidData("invalid Invocation hydration"))?;
        if &checked != run {
            return Err(RunRuntimeCatalogError::InvalidData(
                "Invocation catalogue snapshot differs",
            ));
        }
        let parameters = sqlx::query_scalar(
            "SELECT parameters FROM agent_instance_execution_invocations WHERE run_id=$1",
        )
        .bind(run.id.as_uuid())
        .fetch_one(&mut *tx)
        .await
        .map_err(storage)?;
        tx.commit().await.map_err(storage)?;
        let artifacts = self
            .load_runtime_artifacts(run.release_id.as_uuid())
            .await?;
        Ok(RunRuntimeInput {
            parameters,
            repository_id: None,
            git_ref: None,
            commit_sha: None,
            update_id: None,
            previous_revision_id: None,
            previous_release_id: None,
            previous_parameters: None,
            artifacts,
            previous_artifacts: Vec::new(),
            mailbox_event: None,
        })
    }
}

async fn read_runtime(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    run: &Run,
    payload: serde_json::Value,
    config: Vec<u8>,
    qualified: bool,
) -> Result<RunRow, RunRuntimeCatalogError> {
    Ok(if qualified {
        sqlx::query_as("SELECT execution.* FROM read_qualified_invocation119($1,$2,$3) execution")
            .bind(run.id.as_uuid())
            .bind(payload)
            .bind(config)
            .fetch_one(&mut **tx)
            .await
            .map_err(storage)?
    } else {
        sqlx::query_as("SELECT execution.* FROM read_qualified_invocation117($1,$2,$3) execution")
            .bind(run.id.as_uuid())
            .bind(payload)
            .bind(config)
            .fetch_one(&mut **tx)
            .await
            .map_err(storage)?
    })
}
