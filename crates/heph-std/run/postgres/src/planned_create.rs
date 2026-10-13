//! Atomic worker admission; the ordinary historical create path stays separate.

mod admissions;
pub mod inputs;
pub mod replay;

pub use admissions::consume_for_cleanup;

use run_domain::{FreshRunExecutionProfile, Run, RunCleanupHostId, StartRun};
use run_orchestrator::{CreateRunResult, RepositoryError};
use serde_json::json;
use sqlx::{Postgres, Transaction};
use time::OffsetDateTime;
use vm_trait::VmId;

use crate::{PgRunRepository, errors::storage, model::RunRow};

impl PgRunRepository {
    pub(crate) async fn create_planned_run(
        &self,
        command: &StartRun,
        scope: &RunCleanupHostId,
        vm_id: &VmId,
    ) -> Result<CreateRunResult, RepositoryError> {
        if command.kind == run_domain::RunKind::Invocation {
            return self.consume_invocation(command, scope, vm_id).await;
        }
        if self.execution_profile != Some(FreshRunExecutionProfile::OwnedCanonical) {
            return Err(RepositoryError::InvalidData(
                "owned VM planning requires explicit owned execution profile",
            ));
        }
        if self.vm_scope.as_ref() != Some(scope) {
            return Err(RepositoryError::InvalidData("configured VM owner differs"));
        }
        run_domain::RunCleanupVmTarget::planned(scope.clone(), vm_id.clone()).map_err(storage)?;
        let mut tx = self.pool.begin().await.map_err(storage)?;
        inputs::lock_consumer(&mut tx, command).await?;
        let fresh_inbox = replay::inbox(&mut tx, command).await?;
        let existing = sqlx::query_as::<_, RunRow>("SELECT * FROM runs WHERE id=$1 FOR UPDATE")
            .bind(command.run_id.as_uuid())
            .fetch_optional(&mut *tx)
            .await
            .map_err(storage)?;
        let run = if let Some(row) = existing {
            let run: Run = row.try_into()?;
            crate::execution_profile::require_original(&mut tx, run.id, self.execution_profile)
                .await?;
            replay::existing(&mut tx, command, scope, vm_id, &run).await?;
            sqlx::query_as::<_, RunRow>("SELECT * FROM runs WHERE id=$1")
                .bind(command.run_id.as_uuid())
                .fetch_one(&mut *tx)
                .await
                .map_err(storage)?
                .try_into()?
        } else {
            if vm_id.0 != command.run_id.to_string() {
                return Err(RepositoryError::InvalidData(
                    "fresh VM identity must equal run UUID",
                ));
            }
            if !fresh_inbox {
                return Err(RepositoryError::InvalidData("inbox has no exact run"));
            }
            inputs::validate_pins(&mut tx, command).await?;
            if command.kind == run_domain::RunKind::Update {
                return Err(RepositoryError::InvalidData(
                    "update requires producer admission",
                ));
            }
            insert(&mut tx, command, scope, vm_id).await?;
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
    scope: &RunCleanupHostId,
    vm_id: &VmId,
) -> Result<(), RepositoryError> {
    sqlx::query("INSERT INTO runs(id,instance_id,instance_revision_id,release_id,release_agent_id,attachment_id,run_kind,command_id,state,requires_state,created_at,updated_at,vm_id,vm_provider_namespace,vm_host_id,vm_binding_kind,execution_profile) VALUES($1,$2,$3,$4,$5,$6,$7,$8,'queued',$9,now(),now(),$10,$11,$12,'planned','owned_canonical')")
        .bind(command.run_id.as_uuid()).bind(command.instance_id.as_uuid())
        .bind(command.instance_revision_id.as_uuid()).bind(command.release_id.as_uuid())
        .bind(command.release_agent_id.as_uuid()).bind(command.attachment_id.map(runtime_types::AgentAttachmentId::as_uuid))
        .bind(crate::model::run_kind_name(command.kind)).bind(command.command_id.as_uuid())
        .bind(command.requires_state).bind(&vm_id.0).bind(scope.provider_namespace()).bind(scope.host_id())
        .execute(&mut **tx).await.map_err(storage)?;
    sqlx::query("UPDATE run_requests SET dispatch_state='dispatched' WHERE run_id=$1 AND command_id=$2 AND dispatch_state='pending'")
        .bind(command.run_id.as_uuid()).bind(command.command_id.as_uuid()).execute(&mut **tx).await.map_err(storage)?;
    let now = OffsetDateTime::now_utc();
    PgRunRepository::append_event_tx(
        tx,
        command.run_id,
        "run.queued",
        json!({"run_id":command.run_id,"state":"queued"}),
        now,
    )
    .await?;
    PgRunRepository::append_event_tx(
        tx,
        command.run_id,
        "run.vm_planned",
        json!({"run_id":command.run_id}),
        now,
    )
    .await
}
