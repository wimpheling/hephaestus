use run_domain::{RunKind, StartRun};
use run_orchestrator::RepositoryError;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::errors::storage;

pub async fn lock_consumer(
    tx: &mut Transaction<'_, Postgres>,
    command: &StartRun,
) -> Result<(), RepositoryError> {
    let role: String = sqlx::query_scalar("SELECT current_user::text")
        .fetch_one(&mut **tx)
        .await
        .map_err(storage)?;
    if role != "hephaestus_worker" {
        return Err(RepositoryError::InvalidData(
            "VM admission requires worker role",
        ));
    }
    let instance: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM agent_instances WHERE id=$1 FOR UPDATE")
            .bind(command.instance_id.as_uuid())
            .fetch_optional(&mut **tx)
            .await
            .map_err(storage)?;
    if instance.is_none() {
        return Err(RepositoryError::InvalidData("run consumer is unavailable"));
    }
    Ok(())
}

#[derive(sqlx::FromRow)]
struct Pins {
    requires_state: bool,
    active_revision_id: Option<Uuid>,
    run_gate_open: bool,
    state: String,
}

pub async fn validate_pins(
    tx: &mut Transaction<'_, Postgres>,
    command: &StartRun,
) -> Result<(), RepositoryError> {
    let pins: Pins = sqlx::query_as("SELECT agent.requires_state,instance.active_revision_id,instance.run_gate_open,instance.state FROM agent_instances instance JOIN agent_instance_revisions revision ON revision.instance_id=instance.id AND revision.id=$2 JOIN release_agents agent ON agent.id=revision.release_agent_id AND agent.id=$3 JOIN releases release ON release.id=agent.release_id AND release.id=$4 AND release.state='published' WHERE instance.id=$1 AND NOT EXISTS(SELECT 1 FROM agent_instance_removal_requests WHERE instance_id=instance.id)")
        .bind(command.instance_id.as_uuid()).bind(command.instance_revision_id.as_uuid())
        .bind(command.release_agent_id.as_uuid()).bind(command.release_id.as_uuid())
        .fetch_optional(&mut **tx).await.map_err(storage)?
        .ok_or(RepositoryError::InvalidData("published run pins are unavailable"))?;
    if pins.requires_state != command.requires_state {
        return Err(RepositoryError::InvalidData(
            "new run state requirement differs from released agent",
        ));
    }
    if command.kind == RunKind::Normal
        && (!pins.run_gate_open
            || !matches!(pins.state.as_str(), "active" | "update_rejected")
            || pins.active_revision_id != Some(command.instance_revision_id.as_uuid()))
    {
        return Err(RepositoryError::InvalidData(
            "normal run revision is not open and active",
        ));
    }
    Ok(())
}
