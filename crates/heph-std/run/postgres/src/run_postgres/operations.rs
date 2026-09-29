use run_domain::{CancelRun, InvalidTransition, Run, RunState};
use run_orchestrator::{RepositoryError, StoredVmEvent};
use runtime_types::{LeaseId, RunId, VolumeId};
use serde_json::json;
use time::OffsetDateTime;
use vm_trait::VmExit;

use super::{
    PgRunRepository,
    errors::storage,
    model::{RunRow, outcome_name, parse_state, state_name},
};

impl PgRunRepository {
    pub(super) async fn load_run(&self, run_id: RunId) -> Result<Run, RepositoryError> {
        sqlx::query_as::<_, RunRow>("SELECT * FROM runs WHERE id = $1")
            .bind(run_id.as_uuid())
            .fetch_optional(&self.pool)
            .await
            .map_err(storage)?
            .ok_or(RepositoryError::NotFound(run_id))?
            .try_into()
    }

    pub(super) async fn bind_resources_impl(
        &self,
        run_id: RunId,
        volume_id: Option<VolumeId>,
        lease_id: Option<LeaseId>,
        lease_fencing_token: Option<i64>,
        vm_id: &str,
    ) -> Result<Run, RepositoryError> {
        sqlx::query(
            "UPDATE runs
             SET volume_id = $2, lease_id = $3, lease_fencing_token = $4,
                 vm_id = $5, updated_at = now()
             WHERE id = $1",
        )
        .bind(run_id.as_uuid())
        .bind(volume_id.map(VolumeId::as_uuid))
        .bind(lease_id.map(LeaseId::as_uuid))
        .bind(lease_fencing_token)
        .bind(vm_id)
        .execute(&self.pool)
        .await
        .map_err(storage)?;
        self.load_run(run_id).await
    }

    pub(super) async fn transition_impl(
        &self,
        run_id: RunId,
        next: RunState,
        exit: Option<&VmExit>,
        failure: Option<&str>,
    ) -> Result<Run, RepositoryError> {
        let mut transaction = self.pool.begin().await.map_err(storage)?;
        let current = Self::locked_run(&mut transaction, run_id).await?;
        let current_state = parse_state(&current.state)?;
        if !current_state.can_transition_to(next) {
            return Err(InvalidTransition {
                current: current_state,
                requested: next,
            }
            .into());
        }
        let outcome = next.outcome().map(outcome_name);
        let exit_code = exit.and_then(|value| value.code);
        let exit_signal = exit.and_then(|value| value.signal);
        let now = OffsetDateTime::now_utc();
        sqlx::query(
            "UPDATE runs
             SET state = $2,
                 outcome = COALESCE($3, outcome),
                 exit_code = COALESCE($4, exit_code),
                 exit_signal = COALESCE($5, exit_signal),
                 failure = COALESCE($6, failure),
                 updated_at = $7,
                 state_version = state_version + 1
             WHERE id = $1",
        )
        .bind(run_id.as_uuid())
        .bind(state_name(next))
        .bind(outcome)
        .bind(exit_code)
        .bind(exit_signal)
        .bind(failure)
        .bind(now)
        .execute(&mut *transaction)
        .await
        .map_err(storage)?;
        Self::append_event_tx(
            &mut transaction,
            run_id,
            &format!("run.{}", state_name(next)),
            json!({
                "run_id": run_id,
                "state": state_name(next),
                "outcome": outcome,
                "exit": exit.map(|value| json!({
                    "code": value.code,
                    "signal": value.signal
                })),
                "failure": failure
            }),
            now,
        )
        .await?;
        transaction.commit().await.map_err(storage)?;
        self.load_run(run_id).await
    }

    pub(super) async fn append_vm_event_impl(
        &self,
        run_id: RunId,
        event: StoredVmEvent,
    ) -> Result<(), RepositoryError> {
        let mut transaction = self.pool.begin().await.map_err(storage)?;
        let _run = Self::locked_run(&mut transaction, run_id).await?;
        Self::append_event_tx(
            &mut transaction,
            run_id,
            &event.event_type,
            event.payload,
            event.occurred_at,
        )
        .await?;
        transaction.commit().await.map_err(storage)
    }

    pub(super) async fn request_cancel_impl(
        &self,
        command: &CancelRun,
    ) -> Result<bool, RepositoryError> {
        let mut transaction = self.pool.begin().await.map_err(storage)?;
        let inserted = sqlx::query(
            "INSERT INTO command_inbox (command_id, command_type, payload)
             VALUES ($1, 'cancel_run', $2)
             ON CONFLICT (command_id) DO NOTHING",
        )
        .bind(command.command_id.as_uuid())
        .bind(serde_json::to_value(command).map_err(storage)?)
        .execute(&mut *transaction)
        .await
        .map_err(storage)?
        .rows_affected()
            == 1;
        if inserted {
            let now = OffsetDateTime::now_utc();
            let updated = sqlx::query(
                "UPDATE runs SET cancel_requested_at = COALESCE(cancel_requested_at, $2),
                 updated_at = $2 WHERE id = $1",
            )
            .bind(command.run_id.as_uuid())
            .bind(now)
            .execute(&mut *transaction)
            .await
            .map_err(storage)?
            .rows_affected();
            if updated == 0 {
                return Err(RepositoryError::NotFound(command.run_id));
            }
            sqlx::query("UPDATE command_inbox SET processed_at = $2 WHERE command_id = $1")
                .bind(command.command_id.as_uuid())
                .bind(now)
                .execute(&mut *transaction)
                .await
                .map_err(storage)?;
        }
        transaction.commit().await.map_err(storage)?;
        Ok(inserted)
    }

    pub(super) async fn recoverable_runs_impl(&self) -> Result<Vec<Run>, RepositoryError> {
        sqlx::query_as::<_, RunRow>(
            "SELECT * FROM runs WHERE state <> 'cleaned_up' ORDER BY created_at, id",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(storage)?
        .into_iter()
        .map(TryInto::try_into)
        .collect()
    }
}
