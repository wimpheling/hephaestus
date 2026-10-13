//! Public107 transition locks its immutable consumer before the child Run.

use run_orchestrator::RepositoryError;
use runtime_types::RunId;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::{errors::storage, model::RunRow, repository::PgRunRepository};

impl PgRunRepository {
    pub(super) async fn locked_default_transition_run(
        transaction: &mut Transaction<'_, Postgres>,
        run_id: RunId,
    ) -> Result<RunRow, RepositoryError> {
        // Resolving the immutable parent must not lock the Run: a closure or
        // update admission may already hold the consumer and need this child.
        let parent: Uuid = sqlx::query_scalar("SELECT instance_id FROM runs WHERE id=$1")
            .bind(run_id.as_uuid())
            .fetch_optional(&mut **transaction)
            .await
            .map_err(storage)?
            .ok_or(RepositoryError::NotFound(run_id))?;
        sqlx::query_scalar::<_, Uuid>("SELECT id FROM agent_instances WHERE id=$1 FOR UPDATE")
            .bind(parent)
            .fetch_optional(&mut **transaction)
            .await
            .map_err(storage)?
            .ok_or(RepositoryError::NotFound(run_id))?;
        let current = Self::locked_run(transaction, run_id).await?;
        let exact: bool = sqlx::query_scalar("SELECT instance_id=$2 FROM runs WHERE id=$1")
            .bind(run_id.as_uuid())
            .bind(parent)
            .fetch_one(&mut **transaction)
            .await
            .map_err(storage)?;
        if !exact {
            return Err(RepositoryError::InvalidData(
                "Run consumer changed while locking",
            ));
        }
        // The existing107 progress trigger remains the admission authority;
        // holding its parent prevents transient NOWAIT failure, not denial.
        Ok(current)
    }
}
