use serde_json::{Value, json};
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

pub(crate) async fn emit_publication_diagnostic(
    pool: &PgPool,
    response: &Value,
    gateway_id: Uuid,
    event_id: Uuid,
    run_id: Uuid,
) {
    let (api_publications, page_metadata) = api_publication_diagnostic(response);
    let sql_summary = sql_publication_diagnostic(pool, gateway_id, event_id).await;
    eprintln!(
        "cooking publication inspection diagnostic: {}",
        json!({
            "requestedGatewayId": gateway_id,
            "requestedEventId": event_id,
            "requestedRunId": run_id,
            "api": {
                "publicationCount": api_publications.len(),
                "publications": api_publications,
                "page": page_metadata,
            },
            "sql": sql_summary,
        })
    );
}

pub(crate) fn api_publication_diagnostic(response: &Value) -> (Vec<Value>, Value) {
    let api_publications = response
        .get("publications")
        .and_then(Value::as_array)
        .map(|publications| {
            publications
                .iter()
                .map(|publication| {
                    json!({
                        "id": opaque_id(publication, "id"),
                        "eventId": opaque_id(publication, "eventId"),
                        "deliveryAttemptId": opaque_id(publication, "deliveryAttemptId"),
                        "runId": opaque_id(publication, "runId"),
                        "runState": string_field(publication, "runState"),
                        "runOutcome": string_field(publication, "runOutcome"),
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let page = response.get("page");
    let has_next_page = page
        .and_then(|page| string_field(page, "nextPageToken"))
        .is_some_and(|token| !token.is_empty());
    let page_metadata = json!({
        "stableOrder": page.and_then(|page| string_field(page, "stableOrder")),
        "hasNextPage": has_next_page,
    });
    (api_publications, page_metadata)
}

pub(crate) type PublicationDiagnosticRow = (
    Uuid,
    Option<Uuid>,
    String,
    OffsetDateTime,
    Option<Uuid>,
    Option<i32>,
    Option<Uuid>,
    Option<String>,
    Option<OffsetDateTime>,
    Option<OffsetDateTime>,
    Option<String>,
    Option<String>,
    Option<OffsetDateTime>,
    Option<OffsetDateTime>,
);

pub(crate) async fn sql_publication_diagnostic(
    pool: &PgPool,
    gateway_id: Uuid,
    event_id: Uuid,
) -> Value {
    let sql_rows = sqlx::query_as::<_, PublicationDiagnosticRow>(
        "SELECT publication.id, publication.event_id, publication.outcome,
                publication.accepted_at, attempt.id, attempt.attempt_number,
                attempt.run_id, attempt.state, attempt.created_at,
                attempt.completed_at, run.state, run.outcome, run.created_at,
                run.updated_at
           FROM gateway_mailbox_publications publication
           JOIN gateway_invocations invocation
             ON invocation.id = publication.invocation_id
           LEFT JOIN mailbox_delivery_attempts attempt
             ON attempt.event_id = publication.event_id
           LEFT JOIN runs run ON run.id = attempt.run_id
          WHERE invocation.gateway_id = $1 AND publication.event_id = $2
          ORDER BY publication.id DESC, attempt.attempt_number DESC, attempt.id DESC",
    )
    .bind(gateway_id)
    .bind(event_id)
    .fetch_all(pool)
    .await;
    sql_rows.map_or_else(
        |_| json!({"query": "failed"}),
        |rows| {
            let publication_row_count = rows
                .iter()
                .map(|row| row.0)
                .collect::<std::collections::BTreeSet<_>>()
                .len();
            let attempt_row_count = rows.iter().filter(|row| row.4.is_some()).count();
            json!({
                "publicationRowCount": publication_row_count,
                "attemptRowCount": attempt_row_count,
                "rows": rows.into_iter().map(diagnostic_sql_row).collect::<Vec<_>>(),
            })
        },
    )
}

pub(crate) fn diagnostic_sql_row(
    (
        publication_id,
        publication_event_id,
        publication_outcome,
        publication_accepted_at,
        attempt_id,
        attempt_number,
        attempt_run_id,
        attempt_state,
        attempt_created_at,
        attempt_completed_at,
        run_state,
        run_outcome,
        run_created_at,
        run_updated_at,
    ): PublicationDiagnosticRow,
) -> Value {
    json!({
        "publicationId": publication_id,
        "publicationEventId": publication_event_id,
        "publicationOutcome": publication_outcome,
        "publicationAcceptedAt": diagnostic_timestamp(Some(publication_accepted_at)),
        "attemptId": attempt_id,
        "attemptNumber": attempt_number,
        "attemptRunId": attempt_run_id,
        "attemptState": attempt_state,
        "attemptCreatedAt": diagnostic_timestamp(attempt_created_at),
        "attemptCompletedAt": diagnostic_timestamp(attempt_completed_at),
        "runState": run_state,
        "runOutcome": run_outcome,
        "runCreatedAt": diagnostic_timestamp(run_created_at),
        "runUpdatedAt": diagnostic_timestamp(run_updated_at),
    })
}

pub(crate) fn opaque_id(value: &Value, field: &str) -> Value {
    value
        .get(field)
        .and_then(|field| field.get("value"))
        .and_then(Value::as_str)
        .map_or(Value::Null, |value| Value::String(value.to_owned()))
}

pub(crate) fn string_field(value: &Value, field: &str) -> Option<String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

pub(crate) fn diagnostic_timestamp(value: Option<OffsetDateTime>) -> Value {
    value.map_or(Value::Null, |value| {
        json!({
            "unixSeconds": value.unix_timestamp(),
            "nanoseconds": value.nanosecond(),
        })
    })
}
