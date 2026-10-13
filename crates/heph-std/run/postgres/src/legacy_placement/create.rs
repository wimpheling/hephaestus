use run_domain::{LegacyVmPlacementConsumption, LegacyVmPlacementScope, Run, RunKind, StartRun};
use run_orchestrator::{CreateRunResult, RepositoryError};
use serde_json::json;
use sqlx::{Postgres, Transaction};
use time::OffsetDateTime;

use crate::{PgRunRepository, errors::storage, model::RunRow};

impl PgRunRepository {
    pub(crate) async fn create_legacy_placed_run(
        &self,
        command: &StartRun,
        scope: &LegacyVmPlacementScope,
    ) -> Result<CreateRunResult, RepositoryError> {
        if command.kind == RunKind::Invocation {
            return Err(RepositoryError::InvalidData(
                "Invocation has no Legacy placement",
            ));
        }
        self.require_legacy_scope(scope)?;
        let mut tx = self.pool.begin().await.map_err(storage)?;
        crate::planned_create::inputs::lock_consumer(&mut tx, command).await?;
        let fresh_inbox = crate::planned_create::replay::inbox(&mut tx, command).await?;
        let existing = sqlx::query_as::<_, RunRow>("SELECT * FROM runs WHERE id=$1 FOR UPDATE")
            .bind(command.run_id.as_uuid())
            .fetch_optional(&mut *tx)
            .await
            .map_err(storage)?;
        let run = if let Some(row) = existing {
            let run: Run = row.try_into()?;
            if !crate::model::matches_start_command(&run, command) {
                return Err(RepositoryError::InvalidData(
                    "Legacy existing start command differs",
                ));
            }
            let placement = super::model::load(&mut tx, run.id, scope).await?;
            if placement.command() != command {
                return Err(RepositoryError::InvalidData(
                    "precreated Legacy placement command differs",
                ));
            }
            if placement.consumption().is_none() {
                sqlx::query("SELECT consume_legacy_vm_placement($1,$2,$3,$4,'provision')")
                    .bind(run.id.as_uuid())
                    .bind(command.command_id.as_uuid())
                    .bind(super::namespace(scope)?)
                    .bind(scope.provider_scope().host_id())
                    .execute(&mut *tx)
                    .await
                    .map_err(storage)?;
            } else if placement.consumption() != Some(LegacyVmPlacementConsumption::Provision)
                && run.state != run_domain::RunState::CleanedUp
            {
                return Err(RepositoryError::InvalidData(
                    "cleanup-only Legacy placement cannot authorize start",
                ));
            }
            let closed: bool = sqlx::query_scalar(
                "SELECT volume_acquisition_closed_at IS NOT NULL FROM runs WHERE id=$1",
            )
            .bind(run.id.as_uuid())
            .fetch_one(&mut *tx)
            .await
            .map_err(storage)?;
            if closed && run.state != run_domain::RunState::CleanedUp {
                return Err(RepositoryError::InvalidData(
                    "Legacy start acquisition is permanently closed",
                ));
            }
            run
        } else {
            if !fresh_inbox || command.kind != RunKind::Normal {
                return Err(RepositoryError::InvalidData(
                    "Legacy update needs exact precreated producer placement",
                ));
            }
            crate::planned_create::inputs::validate_pins(&mut tx, command).await?;
            insert(&mut tx, command, scope).await?;
            sqlx::query_as::<_, RunRow>("SELECT * FROM runs WHERE id=$1")
                .bind(command.run_id.as_uuid())
                .fetch_one(&mut *tx)
                .await
                .map_err(storage)?
                .try_into()?
        };
        if fresh_inbox {
            sqlx::query("UPDATE command_inbox SET processed_at=now() WHERE command_id=$1")
                .bind(command.command_id.as_uuid())
                .execute(&mut *tx)
                .await
                .map_err(storage)?;
        }
        tx.commit().await.map_err(storage)?;
        Ok(CreateRunResult {
            run,
            created: fresh_inbox,
        })
    }
}

async fn insert(
    tx: &mut Transaction<'_, Postgres>,
    command: &StartRun,
    scope: &LegacyVmPlacementScope,
) -> Result<(), RepositoryError> {
    sqlx::query("INSERT INTO runs(id,instance_id,instance_revision_id,release_id,release_agent_id,attachment_id,run_kind,command_id,state,requires_state,created_at,updated_at,execution_profile,legacy_vm_placement_required) VALUES($1,$2,$3,$4,$5,$6,'normal',$7,'queued',$8,now(),now(),'legacy_scalar',true)")
        .bind(command.run_id.as_uuid()).bind(command.instance_id.as_uuid())
        .bind(command.instance_revision_id.as_uuid()).bind(command.release_id.as_uuid())
        .bind(command.release_agent_id.as_uuid()).bind(command.attachment_id.map(runtime_types::AgentAttachmentId::as_uuid))
        .bind(command.command_id.as_uuid()).bind(command.requires_state)
        .execute(&mut **tx).await.map_err(storage)?;
    sqlx::query("SELECT record_legacy_vm_placement($1,$2,$3,'normal_request')")
        .bind(command.run_id.as_uuid())
        .bind(super::namespace(scope)?)
        .bind(scope.provider_scope().host_id())
        .execute(&mut **tx)
        .await
        .map_err(storage)?;
    sqlx::query("UPDATE run_requests SET dispatch_state='dispatched' WHERE run_id=$1 AND command_id=$2 AND dispatch_state='pending'")
        .bind(command.run_id.as_uuid()).bind(command.command_id.as_uuid()).execute(&mut **tx).await.map_err(storage)?;
    for kind in ["run.queued", "run.legacy_vm_placement"] {
        PgRunRepository::append_event_tx(
            tx,
            command.run_id,
            kind,
            json!({"run_id": command.run_id}),
            OffsetDateTime::now_utc(),
        )
        .await?;
    }
    Ok(())
}
