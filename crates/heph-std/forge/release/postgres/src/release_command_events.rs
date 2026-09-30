use super::{
    AgentInstanceId, AgentInstanceRevisionId, AuthenticatedIdentity, OffsetDateTime, Postgres,
    RefSelector, ReleaseCommandKey, ReleaseServiceError, Transaction, Uuid, Value, json,
};
use release_domain::TriggerPolicy;

pub fn ref_selector_string(selector: &RefSelector) -> String {
    match selector {
        RefSelector::Exact(value) => value.to_string(),
        RefSelector::Prefix(value) => format!("{value}/*"),
    }
}

pub const fn trigger_policy_name(policy: TriggerPolicy) -> &'static str {
    match policy {
        TriggerPolicy::Push => "push",
        TriggerPolicy::Manual => "manual",
        TriggerPolicy::PushAndManual => "push_and_manual",
    }
}

pub fn decode_hash(value: &str) -> Result<[u8; 32], ReleaseServiceError> {
    if value.len() != 64 {
        return Err(ReleaseServiceError::InvalidStoredData);
    }
    let mut output = [0_u8; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        let pair = std::str::from_utf8(pair).map_err(|_| ReleaseServiceError::InvalidStoredData)?;
        output[index] =
            u8::from_str_radix(pair, 16).map_err(|_| ReleaseServiceError::InvalidStoredData)?;
    }
    Ok(output)
}

pub async fn existing_command(
    tx: &mut Transaction<'_, Postgres>,
    key: ReleaseCommandKey,
    operation: &str,
) -> Result<Option<(Uuid, Option<Uuid>)>, ReleaseServiceError> {
    let row: Option<(String, Uuid, Option<Uuid>)> = sqlx::query_as(
        "SELECT operation, aggregate_id, secondary_id
         FROM release_command_inbox WHERE command_key = $1",
    )
    .bind(key.as_bytes().as_slice())
    .fetch_optional(&mut **tx)
    .await?;
    match row {
        Some((stored, aggregate, secondary)) if stored == operation => {
            Ok(Some((aggregate, secondary)))
        }
        Some(_) => Err(ReleaseServiceError::IdempotencyConflict),
        None => Ok(None),
    }
}

pub async fn record_command(
    tx: &mut Transaction<'_, Postgres>,
    key: ReleaseCommandKey,
    operation: &str,
    aggregate_id: Uuid,
    secondary_id: Option<Uuid>,
    identity: Option<&AuthenticatedIdentity>,
) -> Result<(), ReleaseServiceError> {
    sqlx::query(
        "INSERT INTO release_command_inbox
         (command_key, operation, aggregate_id, secondary_id,
          actor_id, request_id)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(key.as_bytes().as_slice())
    .bind(operation)
    .bind(aggregate_id)
    .bind(secondary_id)
    .bind(identity.map(|value| value.user_id.as_uuid()))
    .bind(identity.map(|value| value.request_id.as_uuid()))
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub async fn append_instance_event(
    tx: &mut Transaction<'_, Postgres>,
    instance_id: AgentInstanceId,
    revision_id: Option<AgentInstanceRevisionId>,
    event_type: &str,
    identity: &AuthenticatedIdentity,
    payload: Value,
) -> Result<(), ReleaseServiceError> {
    sqlx::query(
        "INSERT INTO agent_instance_events
         (id, instance_id, revision_id, event_type, actor_id, request_id,
          payload)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(Uuid::new_v4())
    .bind(instance_id.as_uuid())
    .bind(revision_id.map(AgentInstanceRevisionId::as_uuid))
    .bind(event_type)
    .bind(identity.user_id.as_uuid())
    .bind(identity.request_id.as_uuid())
    .bind(payload)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub async fn append_event(
    tx: &mut Transaction<'_, Postgres>,
    aggregate_id: Uuid,
    subject: &str,
    event_type: &str,
    mut payload: Value,
) -> Result<(), ReleaseServiceError> {
    let event_id = Uuid::new_v4();
    enrich_message_payload(&mut payload, event_id);
    sqlx::query(
        "INSERT INTO outbox
         (id, aggregate_type, aggregate_id, subject, event_type,
          payload, occurred_at)
         VALUES ($1, 'release', $2, $3, $4, $5, $6)",
    )
    .bind(event_id)
    .bind(aggregate_id)
    .bind(subject)
    .bind(event_type)
    .bind(payload)
    .bind(OffsetDateTime::now_utc())
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub fn enrich_message_payload(payload: &mut Value, event_id: Uuid) {
    let Some(object) = payload.as_object_mut() else {
        return;
    };
    object
        .entry(String::from("schema_version"))
        .or_insert_with(|| json!(1));
    object
        .entry(String::from("message_id"))
        .or_insert_with(|| json!(event_id));
    object
        .entry(String::from("idempotency_key"))
        .or_insert_with(|| json!(event_id));
    object
        .entry(String::from("request_id"))
        .or_insert(Value::Null);
    object
        .entry(String::from("trace_id"))
        .or_insert(Value::Null);
}

pub fn is_update_admission_generation_conflict(error: &sqlx::Error) -> bool {
    let Some(database_error) = error.as_database_error() else {
        return false;
    };
    database_error.code().as_deref() == Some("23505")
        && matches!(
            database_error.constraint(),
            Some("runs_pkey" | "runs_exact_revision_unique" | "runs_command_id_key")
        )
}
