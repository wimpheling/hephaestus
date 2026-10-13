//! Independent worker tombstone + unique actual Applied history in one RRRO.
use super::super::decoding;
use super::{Reader, applied, closure, encoding, role, storage};
use identity_domain::{RequestId, UserId};
use recipe_application::{
    DeploymentCommandId, DeploymentError, PreparedInstanceDeleteCompletedHistory,
    PreparedInstanceDeleteHistory, PreparedInstanceDeleteHistoryState,
    PreparedInstanceDeleteRequest, PreparedInstanceTerminalProgress,
    PreparedInstanceTerminalPublisher,
};
use serde_json::{Value, json};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

pub(super) async fn observe(
    reader: &Reader,
    request: &PreparedInstanceDeleteRequest,
) -> Result<PreparedInstanceDeleteCompletedHistory, DeploymentError> {
    let mut tx = reader
        .worker
        .begin_with("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .await
        .map_err(storage)?;
    let result=async{
        role(&mut tx,"hephaestus_worker").await?;
        let raw=closure::worker(&mut tx,request).await?;
        let expected=encoding::request(reader,request,&raw)?;
        let rows:Vec<(Uuid,i64,Value)>=sqlx::query_as("SELECT terminal_operation_id,after_instance_version,terminal_history FROM observe_prepared_instance_deleted143($1,$2)")
            .bind(&expected).bind(encoding::configuration(reader)?).fetch_all(&mut *tx).await.map_err(storage)?;
        if rows.len()!=1 || rows[0].0.is_nil(){return Err(DeploymentError::ReconciliationRequired);}
        let (operation,after,terminal)=&rows[0];
        let state=applied::observe(&mut tx,request).await?;
        let history=decode_history(&mut tx,request,terminal).await?;
        if history.state()!=PreparedInstanceDeleteHistoryState::Applied || history.observed_progress()!=&state
            || terminal["terminal_operation_id"]!=json!(operation)
            || terminal["after_instance_version"]!=json!(after) {
            return Err(DeploymentError::ReconciliationRequired);
        }
        PreparedInstanceDeleteCompletedHistory::new(history.original().clone(),state,history.publisher_admission_command())
    }.await;
    let rollback = tx.rollback().await.map_err(storage);
    match result {
        Err(primary) => Err(primary),
        Ok(value) => rollback.map(|()| value),
    }
}

pub(super) async fn decode_history(
    tx: &mut Transaction<'_, Postgres>,
    request: &PreparedInstanceDeleteRequest,
    raw: &Value,
) -> Result<PreparedInstanceDeleteHistory, DeploymentError> {
    let entries = decoding::terminal_history_entries(&json!({"terminal":[raw]}))?;
    if entries.len() != 1 {
        return Err(DeploymentError::ReconciliationRequired);
    }
    let entry = &entries[0];
    if entry.resource != request.effect().resource
        || entry.claim != crate::execution_rows::ClaimWire::new(request.effect())?
        || entry.before != Some(request.expected_instance_version())
    {
        return Err(DeploymentError::IntentMismatch);
    }
    let publisher = if let Some(value) = &entry.publisher {
        let command = decoding::terminal_command(tx, parse_uuid(value, "command_id")?).await?;
        Some(PreparedInstanceTerminalPublisher::new(
            request.effect(),
            command,
            DeploymentCommandId::from_uuid(parse_uuid(value, "command_id")?)?,
            UserId::from_uuid(parse_uuid(value, "actor_id")?),
            RequestId::from_uuid(parse_uuid(value, "request_id")?),
            if value["admission_command_id"].is_null() {
                None
            } else {
                Some(DeploymentCommandId::from_uuid(parse_uuid(
                    value,
                    "admission_command_id",
                )?)?)
            },
        )?)
    } else {
        None
    };
    let state = PreparedInstanceTerminalProgress::new(
        entry.state.progress()?,
        crate::hydration::unsigned(entry.state.generation)?,
        entry.state.last_action,
    )?;
    PreparedInstanceDeleteHistory::from_observed_parts(
        request.clone(),
        state,
        entry.operation,
        entry.after,
        publisher,
    )
}
fn parse_uuid(value: &Value, name: &str) -> Result<Uuid, DeploymentError> {
    let id = value[name]
        .as_str()
        .ok_or(DeploymentError::ReconciliationRequired)?
        .parse::<Uuid>()
        .map_err(|_| DeploymentError::ReconciliationRequired)?;
    if id.is_nil() {
        return Err(DeploymentError::ReconciliationRequired);
    }
    Ok(id)
}
