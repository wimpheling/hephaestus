use run_domain::{Run, RunState, StartRun};
use run_orchestrator::{CreateRunResult, RepositoryError};
use serde_json::json;
use time::OffsetDateTime;

use super::model::{RunRow, matches_start_command};
use super::{PgRunRepository, errors::storage};

impl PgRunRepository {
    pub(super) async fn create_run_impl(
        &self,
        command: &StartRun,
    ) -> Result<CreateRunResult, RepositoryError> {
        require_supported_kind(command.kind)?;
        let mut transaction = self.pool.begin().await.map_err(storage)?;
        let fresh_inbox = crate::planned_create::replay::inbox(&mut transaction, command).await?;
        if !fresh_inbox {
            let row = sqlx::query_as::<_, RunRow>("SELECT * FROM runs WHERE command_id = $1")
                .bind(command.command_id.as_uuid())
                .fetch_one(&mut *transaction)
                .await
                .map_err(storage)?;
            crate::execution_profile::require_original(
                &mut transaction,
                command.run_id,
                self.execution_profile,
            )
            .await?;
            transaction.commit().await.map_err(storage)?;
            return Ok(CreateRunResult {
                run: row.try_into()?,
                created: false,
            });
        }
        if let Some(row) = sqlx::query_as::<_, RunRow>("SELECT * FROM runs WHERE id = $1")
            .bind(command.run_id.as_uuid())
            .fetch_optional(&mut *transaction)
            .await
            .map_err(storage)?
        {
            let run: Run = row.try_into()?;
            if !matches_start_command(&run, command) || run.state != RunState::Queued {
                return Err(RepositoryError::InvalidData(
                    "precreated update run does not match start command",
                ));
            }
            crate::execution_profile::require_original(
                &mut transaction,
                command.run_id,
                self.execution_profile,
            )
            .await?;
            let now = OffsetDateTime::now_utc();
            sqlx::query("UPDATE command_inbox SET processed_at = $2 WHERE command_id = $1")
                .bind(command.command_id.as_uuid())
                .bind(now)
                .execute(&mut *transaction)
                .await
                .map_err(storage)?;
            transaction.commit().await.map_err(storage)?;
            return Ok(CreateRunResult { run, created: true });
        }

        let now = OffsetDateTime::now_utc();
        crate::execution_profile::insert_scalar(
            &mut transaction,
            command,
            self.execution_profile,
            now,
        )
        .await?;
        sqlx::query(
            "UPDATE run_requests
             SET dispatch_state = 'dispatched'
             WHERE run_id = $1 AND command_id = $2
               AND dispatch_state = 'pending'",
        )
        .bind(command.run_id.as_uuid())
        .bind(command.command_id.as_uuid())
        .execute(&mut *transaction)
        .await
        .map_err(storage)?;
        Self::append_event_tx(
            &mut transaction,
            command.run_id,
            "run.queued",
            json!({"run_id": command.run_id, "state": "queued"}),
            now,
        )
        .await?;
        sqlx::query("UPDATE command_inbox SET processed_at = $2 WHERE command_id = $1")
            .bind(command.command_id.as_uuid())
            .bind(now)
            .execute(&mut *transaction)
            .await
            .map_err(storage)?;
        transaction.commit().await.map_err(storage)?;
        Ok(CreateRunResult {
            run: self.load_run(command.run_id).await?,
            created: true,
        })
    }
}

const fn require_supported_kind(kind: run_domain::RunKind) -> Result<(), RepositoryError> {
    match kind {
        run_domain::RunKind::Normal | run_domain::RunKind::Update => Ok(()),
        run_domain::RunKind::Invocation => Err(RepositoryError::InvalidData(
            "Invocation creation is unsupported",
        )),
    }
}
