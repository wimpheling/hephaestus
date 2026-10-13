use run_domain::{Run, RunCleanupHostId, RunState, StartRun};
use run_orchestrator::RepositoryError;
use sqlx::{Postgres, Transaction};
use vm_trait::VmId;

use crate::{errors::storage, model::matches_start_command};

pub async fn inbox(
    tx: &mut Transaction<'_, Postgres>,
    command: &StartRun,
) -> Result<bool, RepositoryError> {
    let payload = serde_json::to_value(command).map_err(storage)?;
    let inserted = sqlx::query("INSERT INTO command_inbox(command_id,command_type,payload) VALUES($1,'start_run',$2) ON CONFLICT(command_id) DO NOTHING")
        .bind(command.command_id.as_uuid()).bind(&payload).execute(&mut **tx).await.map_err(storage)?;
    let exact: bool = sqlx::query_scalar("SELECT command_type='start_run' AND payload=$2 FROM command_inbox WHERE command_id=$1 FOR UPDATE")
        .bind(command.command_id.as_uuid()).bind(payload).fetch_one(&mut **tx).await.map_err(storage)?;
    if !exact {
        return Err(RepositoryError::InvalidData(
            "immutable start inbox input differs",
        ));
    }
    Ok(inserted.rows_affected() == 1)
}

#[derive(sqlx::FromRow)]
struct Plan {
    vm_provider_namespace: Option<String>,
    vm_host_id: Option<String>,
    vm_binding_kind: Option<String>,
    volume_acquisition_closed_at: Option<time::OffsetDateTime>,
    vm_plan_admission_required: bool,
}

pub async fn existing(
    tx: &mut Transaction<'_, Postgres>,
    command: &StartRun,
    scope: &RunCleanupHostId,
    vm_id: &VmId,
    run: &Run,
) -> Result<(), RepositoryError> {
    if !matches_start_command(run, command) {
        return Err(RepositoryError::InvalidData("exact run command differs"));
    }
    let plan: Plan = sqlx::query_as("SELECT vm_provider_namespace,vm_host_id,vm_binding_kind,volume_acquisition_closed_at,vm_plan_admission_required FROM runs WHERE id=$1")
        .bind(run.id.as_uuid()).fetch_one(&mut **tx).await.map_err(storage)?;
    if plan.vm_binding_kind.is_none() {
        if run.state == RunState::CleanedUp {
            // Existing grandfathered terminal state is not new ownership proof.
            return Ok(());
        }
        if !plan.vm_plan_admission_required {
            return Err(RepositoryError::InvalidData(
                "historical NULL scope has no IO admission",
            ));
        }
        super::inputs::validate_pins(tx, command).await?;
        return super::admissions::consume(tx, command, scope, vm_id).await;
    }
    if plan.vm_provider_namespace.as_deref() != Some(scope.provider_namespace())
        || plan.vm_host_id.as_deref() != Some(scope.host_id())
        || run.vm_id.as_deref() != Some(vm_id.0.as_str())
    {
        return Err(RepositoryError::InvalidData(
            "persisted VM owner or identity differs",
        ));
    }
    if plan.volume_acquisition_closed_at.is_some() && run.state != RunState::CleanedUp {
        return Err(RepositoryError::InvalidData(
            "run acquisition is permanently closed",
        ));
    }
    Ok(())
}
