use event_application::CommittedMutation;
use identity_domain::AuthenticatedIdentity;
use recipe_application::{
    AdmissionDisposition, CommandIdentity, CommandReceipt, DeploymentError, DeploymentSnapshot,
};
use release_domain::ContentHash;
use sqlx::{Postgres, Transaction};

use crate::{
    hydration::{digest, unsigned},
    repository_error,
    rows::ReceiptRow,
};

pub async fn find(
    tx: &mut Transaction<'_, Postgres>,
    command: CommandIdentity,
) -> Result<Option<ReceiptRow>, DeploymentError> {
    sqlx::query_as(
        "SELECT id, deployment_id, project_id, input_hash, receipt_lifecycle, receipt_version,
                event_id, event_cursor, event_aggregate_version FROM recipe_deployment_commands
         WHERE actor_id = $1 AND operation = $2 AND idempotency_id = $3",
    )
    .bind(command.actor_id().as_uuid())
    .bind(command.operation().as_str())
    .bind(command.idempotency_id().as_uuid())
    .fetch_optional(&mut **tx)
    .await
    .map_err(repository_error)
}

pub fn restore(
    row: &ReceiptRow,
    command: CommandIdentity,
    snapshot: &DeploymentSnapshot,
    input_hash: ContentHash,
) -> Result<CommandReceipt, DeploymentError> {
    if row.id != command.id().as_uuid()
        || row.deployment_id != snapshot.intent.id().as_uuid()
        || row.project_id != snapshot.intent.project_id().as_uuid()
        || row.input_hash.as_slice() != input_hash.as_bytes()
    {
        return Err(DeploymentError::InputConflict);
    }
    Ok(CommandReceipt {
        command,
        deployment_id: snapshot.intent.id(),
        input_hash: digest(&row.input_hash)?,
        lifecycle: row.receipt_lifecycle.parse()?,
        deployment_version: unsigned(row.receipt_version)?,
        event: CommittedMutation {
            event_id: row.event_id,
            scope_kind: "project".into(),
            scope_id: row.project_id,
            cursor: row.event_cursor,
            aggregate_version: row.event_aggregate_version,
        },
    })
}

pub async fn append(
    tx: &mut Transaction<'_, Postgres>,
    identity: &AuthenticatedIdentity,
    command: CommandIdentity,
    snapshot: &DeploymentSnapshot,
    input_hash: ContentHash,
    organization: uuid::Uuid,
) -> Result<CommandReceipt, DeploymentError> {
    let (event_id, cursor, aggregate_version): (uuid::Uuid, i64, i64) = sqlx::query_as(
        "SELECT event_id, cursor, aggregate_version FROM append_application_event(
            $1, 'project', $2, 'project', $2, 'project.changed', 'updated', NULL, $3, NULL)",
    )
    .bind(command.idempotency_id().as_uuid())
    .bind(snapshot.intent.project_id().as_uuid())
    .bind(organization)
    .fetch_one(&mut **tx)
    .await
    .map_err(repository_error)?;
    sqlx::query(
        "INSERT INTO recipe_deployment_commands (id, deployment_id, project_id, actor_id, operation,
            idempotency_id, input_hash, first_request_id, receipt_lifecycle, receipt_version,
            event_id, event_cursor, event_aggregate_version) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)",
    ).bind(command.id().as_uuid()).bind(snapshot.intent.id().as_uuid()).bind(snapshot.intent.project_id().as_uuid())
        .bind(identity.user_id.as_uuid()).bind(command.operation().as_str()).bind(command.idempotency_id().as_uuid())
        .bind(input_hash.as_bytes().as_slice()).bind(identity.request_id.as_uuid()).bind(snapshot.lifecycle.as_str())
        .bind(i64::try_from(snapshot.version).map_err(|_| DeploymentError::StaleClaim)?)
        .bind(event_id).bind(cursor).bind(aggregate_version).execute(&mut **tx).await.map_err(super::persistence::write_error)?;
    Ok(CommandReceipt {
        command,
        deployment_id: snapshot.intent.id(),
        input_hash,
        lifecycle: snapshot.lifecycle,
        deployment_version: snapshot.version,
        event: CommittedMutation {
            event_id,
            scope_kind: "project".into(),
            scope_id: snapshot.intent.project_id().as_uuid(),
            cursor,
            aggregate_version,
        },
    })
}

pub async fn attempt(
    tx: &mut Transaction<'_, Postgres>,
    identity: &AuthenticatedIdentity,
    receipt: &CommandReceipt,
    project: forge_domain::ProjectId,
    disposition: AdmissionDisposition,
) -> Result<(), DeploymentError> {
    let disposition = match disposition {
        AdmissionDisposition::Created => "created",
        AdmissionDisposition::Resume => "resume",
        AdmissionDisposition::Replay => "replay",
    };
    sqlx::query(
        "INSERT INTO recipe_deployment_admission_attempts (id, command_id, deployment_id,
            project_id, actor_id, request_id, disposition) VALUES ($1,$2,$3,$4,$5,$6,$7)",
    )
    .bind(uuid::Uuid::new_v4())
    .bind(receipt.command.id().as_uuid())
    .bind(receipt.deployment_id.as_uuid())
    .bind(project.as_uuid())
    .bind(identity.user_id.as_uuid())
    .bind(identity.request_id.as_uuid())
    .bind(disposition)
    .execute(&mut **tx)
    .await
    .map_err(repository_error)?;
    Ok(())
}
