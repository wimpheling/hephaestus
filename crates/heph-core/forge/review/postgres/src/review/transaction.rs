//! Shared authorization and transaction helpers for review commands.

use authz_domain::{ObjectRef, Permission, Subject};
use authz_postgres::{PostgresMelangeAuthorizer, audit_decision};
use review_domain::{ControlCommand, ControlRequestId};
use review_service::ReviewRepositoryError;
use runtime_types::RunId;
use serde_json::Value;
use sqlx::{Postgres, Transaction};
use time::OffsetDateTime;
use uuid::Uuid;

use super::support::{db_error, infrastructure};

pub async fn authorize(
    authorizer: &PostgresMelangeAuthorizer,
    transaction: &mut Transaction<'_, Postgres>,
    command: &ControlCommand,
    permission: Permission,
    object: ObjectRef,
) -> Result<authz_domain::AuthorizationDecision, ReviewRepositoryError> {
    let decision = authorizer
        .check(
            transaction,
            Subject::User(command.actor_id),
            permission,
            object,
        )
        .await
        .map_err(|error| infrastructure(error.to_string()))?;
    audit_decision(
        transaction,
        command.actor_id,
        permission,
        object,
        decision,
        command.request_id,
    )
    .await
    .map_err(db_error)?;
    Ok(decision)
}

pub async fn set_actor(
    transaction: &mut Transaction<'_, Postgres>,
    command: &ControlCommand,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "SELECT set_config('hephaestus.actor_id', $1, true),
                set_config('hephaestus.subject_type', 'user', true),
                set_config('hephaestus.request_id', $2, true)",
    )
    .bind(command.actor_id.to_string())
    .bind(command.request_id.to_string())
    .execute(&mut **transaction)
    .await
    .map(|_| ())
}

pub async fn close_denied(
    transaction: &mut Transaction<'_, Postgres>,
    id: ControlRequestId,
) -> Result<(), ReviewRepositoryError> {
    sqlx::query(
        "UPDATE control_requests
         SET state = 'failed', diagnostics = jsonb_build_array(
             jsonb_build_object('code', 'authorization_denied')),
             processed_at = now() WHERE id = $1",
    )
    .bind(id.as_uuid())
    .execute(&mut **transaction)
    .await
    .map(|_| ())
    .map_err(db_error)
}

pub async fn close_rejected(
    transaction: &mut Transaction<'_, Postgres>,
    id: ControlRequestId,
    code: &str,
) -> Result<(), ReviewRepositoryError> {
    sqlx::query(
        "UPDATE control_requests
         SET state = 'failed', diagnostics = jsonb_build_array(
             jsonb_build_object('code', $2)),
             processed_at = now() WHERE id = $1",
    )
    .bind(id.as_uuid())
    .bind(code)
    .execute(&mut **transaction)
    .await
    .map(|_| ())
    .map_err(db_error)
}

pub async fn complete_control(
    transaction: &mut Transaction<'_, Postgres>,
    id: ControlRequestId,
) -> Result<(), ReviewRepositoryError> {
    sqlx::query(
        "UPDATE control_requests SET state = 'completed', processed_at = now() WHERE id = $1",
    )
    .bind(id.as_uuid())
    .execute(&mut **transaction)
    .await
    .map(|_| ())
    .map_err(db_error)
}

pub async fn insert_outbox(
    transaction: &mut Transaction<'_, Postgres>,
    aggregate_type: &str,
    aggregate_id: Uuid,
    subject: &str,
    event_type: &str,
    payload: Value,
) -> Result<(), ReviewRepositoryError> {
    sqlx::query(
        "INSERT INTO outbox
         (id, aggregate_type, aggregate_id, subject, event_type, payload, occurred_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(Uuid::new_v4())
    .bind(aggregate_type)
    .bind(aggregate_id)
    .bind(subject)
    .bind(event_type)
    .bind(payload)
    .bind(OffsetDateTime::now_utc())
    .execute(&mut **transaction)
    .await
    .map(|_| ())
    .map_err(db_error)
}

pub async fn append_run_event(
    transaction: &mut Transaction<'_, Postgres>,
    run_id: RunId,
    event_type: &str,
    payload: Value,
) -> Result<(), ReviewRepositoryError> {
    sqlx::query("SELECT id FROM runs WHERE id = $1 FOR UPDATE")
        .bind(run_id.as_uuid())
        .execute(&mut **transaction)
        .await
        .map_err(db_error)?;
    sqlx::query(
        "INSERT INTO run_events
         (id, run_id, sequence, event_type, payload, occurred_at)
         SELECT $1, $2, COALESCE(max(sequence), 0) + 1, $3, $4, $5
         FROM run_events WHERE run_id = $2",
    )
    .bind(Uuid::new_v4())
    .bind(run_id.as_uuid())
    .bind(event_type)
    .bind(payload)
    .bind(OffsetDateTime::now_utc())
    .execute(&mut **transaction)
    .await
    .map(|_| ())
    .map_err(db_error)
}
