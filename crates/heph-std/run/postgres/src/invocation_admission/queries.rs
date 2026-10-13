use run_domain::{RunCleanupHostId, StartRun};
use run_orchestrator::RepositoryError;
use sqlx::{Postgres, Transaction};
use vm_trait::VmId;

use crate::{errors::storage, model::RunRow};

pub(super) async fn consume(
    tx: &mut Transaction<'_, Postgres>,
    command: &StartRun,
    scope: &RunCleanupHostId,
    vm: &VmId,
    payload: serde_json::Value,
    config: Vec<u8>,
    qualified: bool,
) -> Result<RunRow, RepositoryError> {
    Ok(if qualified {
        sqlx::query_as(
            "SELECT execution.* FROM consume_qualified_invocation119($1,$2,$3,$4,$5,$6) execution",
        )
        .bind(command.run_id.as_uuid())
        .bind(payload)
        .bind(config)
        .bind(scope.provider_namespace())
        .bind(scope.host_id())
        .bind(&vm.0)
        .fetch_one(&mut **tx)
        .await
        .map_err(storage)?
    } else {
        sqlx::query_as(
            "SELECT execution.* FROM consume_qualified_invocation117($1,$2,$3,$4,$5,$6) execution",
        )
        .bind(command.run_id.as_uuid())
        .bind(payload)
        .bind(config)
        .bind(scope.provider_namespace())
        .bind(scope.host_id())
        .bind(&vm.0)
        .fetch_one(&mut **tx)
        .await
        .map_err(storage)?
    })
}
