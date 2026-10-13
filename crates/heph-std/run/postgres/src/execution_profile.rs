use run_domain::{FreshRunExecutionProfile, StartRun};
use run_orchestrator::RepositoryError;
use runtime_types::{AgentAttachmentId, RunId};
use sqlx::{Postgres, Transaction};

use crate::{errors::storage, model::run_kind_name};
use time::OffsetDateTime;

pub async fn require_original(
    tx: &mut Transaction<'_, Postgres>,
    run_id: RunId,
    configured: Option<FreshRunExecutionProfile>,
) -> Result<(), RepositoryError> {
    let Some(profile) = configured else {
        return Ok(());
    };
    let original: String = sqlx::query_scalar("SELECT execution_profile FROM runs WHERE id=$1")
        .bind(run_id.as_uuid())
        .fetch_one(&mut **tx)
        .await
        .map_err(storage)?;
    if original != profile.as_str() {
        return Err(RepositoryError::InvalidData(
            "persisted Run profile differs; replay cannot promote history",
        ));
    }
    Ok(())
}

pub async fn insert_scalar(
    tx: &mut Transaction<'_, Postgres>,
    command: &StartRun,
    configured: Option<FreshRunExecutionProfile>,
    now: OffsetDateTime,
) -> Result<(), RepositoryError> {
    if configured == Some(FreshRunExecutionProfile::OwnedCanonical) {
        return Err(RepositoryError::InvalidData(
            "owned fresh Run requires exact VM planning",
        ));
    }
    let insert = if configured.is_some() {
        sqlx::query(
            "INSERT INTO runs
                 (id, instance_id, instance_revision_id, release_id,
                  release_agent_id, attachment_id, run_kind, command_id, state,
                  requires_state, created_at, updated_at, execution_profile)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'queued', $9, $10, $10, $11)",
        )
    } else {
        sqlx::query(
            "INSERT INTO runs
             (id, instance_id, instance_revision_id, release_id,
              release_agent_id, attachment_id, run_kind, command_id, state,
              requires_state, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'queued', $9, $10, $10)",
        )
    };
    let insert = insert
        .bind(command.run_id.as_uuid())
        .bind(command.instance_id.as_uuid())
        .bind(command.instance_revision_id.as_uuid())
        .bind(command.release_id.as_uuid())
        .bind(command.release_agent_id.as_uuid())
        .bind(command.attachment_id.map(AgentAttachmentId::as_uuid))
        .bind(run_kind_name(command.kind))
        .bind(command.command_id.as_uuid())
        .bind(command.requires_state)
        .bind(now);
    let insert = if let Some(profile) = configured {
        insert.bind(profile.as_str())
    } else {
        insert
    };
    insert.execute(&mut **tx).await.map_err(storage)?;
    Ok(())
}
