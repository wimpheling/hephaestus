//! Invocation terminal result qualification in the owning transition transaction.

use run_domain::RunState;
use run_orchestrator::RepositoryError;
use sqlx::{Postgres, Transaction};
use vm_trait::VmExit;

use crate::{PgRunRepository, errors::storage, model::RunRow};

impl PgRunRepository {
    // The caller already holds the existing early reservations and owning row.
    pub(crate) async fn qualified_invocation_terminal_state(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        current: &RunRow,
        next: RunState,
        exit: Option<&VmExit>,
        failure: Option<&str>,
    ) -> Result<RunState, RepositoryError> {
        if next != RunState::Succeeded || !current.is_invocation() {
            return Ok(next);
        }
        if !matches!(
            exit,
            Some(VmExit {
                code: Some(0),
                signal: None
            })
        ) || failure.is_some()
        {
            return Err(RepositoryError::InvalidData(
                "Invocation success requires actual exit zero without signal or failure",
            ));
        }
        if !self.invocation_lease_runtime {
            return Err(RepositoryError::InvalidData(
                "Invocation success requires explicit119 composition",
            ));
        }
        let configuration = self.invocation_configuration()?;
        let config = serde_json::to_vec(&configuration.canonical_json()).map_err(storage)?;
        let effective: String = sqlx::query_scalar(
            "SELECT qualify_invocation119_terminal_result($1,$2,$3,$4,$5,$6,$7,$8)",
        )
        .bind(current.run_id().as_uuid())
        .bind(config)
        .bind(configuration.provider_scope().namespace())
        .bind(configuration.provider_scope().host_id())
        .bind(exit.is_some())
        .bind(exit.and_then(|value| value.code))
        .bind(exit.and_then(|value| value.signal))
        .bind(failure)
        .fetch_one(&mut **tx)
        .await
        .map_err(storage)?;
        match effective.as_str() {
            "succeeded" => Ok(RunState::Succeeded),
            "cancelled" => Ok(RunState::Cancelled),
            _ => Err(RepositoryError::InvalidData(
                "Invocation terminal qualifier returned an unexpected state",
            )),
        }
    }
}
