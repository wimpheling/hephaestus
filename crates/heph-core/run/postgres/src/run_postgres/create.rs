use run_domain::{Run, RunState, StartRun};
use run_orchestrator::{CreateRunResult, RepositoryError};
use runtime_types::AgentAttachmentId;
use serde_json::json;
use time::OffsetDateTime;

use super::model::{RunRow, matches_start_command, run_kind_name};
use super::{PgRunRepository, errors::storage};

impl PgRunRepository {
    pub(super) async fn create_run_impl(
        &self,
        command: &StartRun,
    ) -> Result<CreateRunResult, RepositoryError> {
        let mut transaction = self.pool.begin().await.map_err(storage)?;
        let payload = serde_json::to_value(command).map_err(storage)?;
        let inbox = sqlx::query(
            "INSERT INTO command_inbox (command_id, command_type, payload)
             VALUES ($1, 'start_run', $2)
             ON CONFLICT (command_id) DO NOTHING",
        )
        .bind(command.command_id.as_uuid())
        .bind(payload)
        .execute(&mut *transaction)
        .await
        .map_err(storage)?;
        if inbox.rows_affected() == 0 {
            let row = sqlx::query_as::<_, RunRow>("SELECT * FROM runs WHERE command_id = $1")
                .bind(command.command_id.as_uuid())
                .fetch_one(&mut *transaction)
                .await
                .map_err(storage)?;
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
        sqlx::query(
            "INSERT INTO runs
             (id, instance_id, instance_revision_id, release_id,
              release_agent_id, attachment_id, run_kind, command_id, state,
              requires_state, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'queued', $9, $10, $10)",
        )
        .bind(command.run_id.as_uuid())
        .bind(command.instance_id.as_uuid())
        .bind(command.instance_revision_id.as_uuid())
        .bind(command.release_id.as_uuid())
        .bind(command.release_agent_id.as_uuid())
        .bind(command.attachment_id.map(AgentAttachmentId::as_uuid))
        .bind(run_kind_name(command.kind))
        .bind(command.command_id.as_uuid())
        .bind(command.requires_state)
        .bind(now)
        .execute(&mut *transaction)
        .await
        .map_err(storage)?;
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
