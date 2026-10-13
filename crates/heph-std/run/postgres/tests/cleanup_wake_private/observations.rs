use serde_json::{Map, Value};
use sqlx::PgPool;

#[path = "snapshot_queries.rs"]
mod snapshot_queries;

// Whitelist table identities; compare complete rows, including events/outboxes.
pub async fn rows(pool: &PgPool) -> Value {
    let mut result = Map::new();
    for table in [
        "runs",
        "run_events",
        "agent_instance_events",
        "run_invocation_consumptions",
        "agent_instance_execution_invocations",
        "agent_instances",
        "agent_instance_state_volumes",
        "agent_instance_volume_leases",
        "agent_instance_revision_volume_bindings",
        "agent_instance_volume_mount_grants",
        "agent_instance_volume_mount_revocations",
        "run_cleanup_targets",
        "run_cleanup_receipts",
        "run_cleanup_target_leases",
        "run_cleanup_completions",
        "recipe_deployments",
        "recipe_deployment_resources",
        "recipe_deployment_commands",
        "recipe_effect_attempts",
        "recipe_effect_verifications",
        "recipe_execution_transitions",
        "recipe_deployment_execution_preparations",
        "recipe_prepared_instance_imports",
        "recipe_remove_resume_admissions",
        "agent_instance_removal_requests",
        "release_command_inbox",
        "command_inbox",
        "application_events",
        "product_event_outbox",
        "outbox",
        "authorization_audit_events",
    ] {
        let value: Value = snapshot_queries::table(table)
            .fetch_one(pool)
            .await
            .unwrap();
        result.insert(table.to_owned(), value);
    }
    Value::Object(result)
}
pub fn held(error: &run_orchestrator::RepositoryError) {
    assert!(
        matches!(
            error,
            run_orchestrator::RepositoryError::InvalidData(
                "cleanup wake history is held or contradictory"
            )
        ),
        "{error:?}"
    );
}
pub fn code(error: &sqlx::Error) -> Option<String> {
    error
        .as_database_error()
        .and_then(|e| e.code().map(std::borrow::Cow::into_owned))
}
pub async fn preserved_closure(pool: &PgPool) -> Value {
    let all = rows(pool).await;
    serde_json::json!({"removal":all["agent_instance_removal_requests"], "inbox":all["release_command_inbox"], "cancel_outbox":all["outbox"], "attempts":all["recipe_effect_attempts"]})
}

// The unchanged target writer appends one cleanup event. Its acquisition UPDATE
// emits three scoped updated events; the target INSERT emits one created event.
// Each application event gets one durable product outbox row. run_events itself
// is excluded from application publication by the unchanged public migration.
pub async fn preparation_events(
    pool: &PgPool,
    before: &Value,
    after: &Value,
    run: runtime_types::RunId,
    lease_count: usize,
) {
    let run_id = serde_json::json!(run);
    let owned: Vec<_> = after["runs"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["id"] == run_id)
        .collect();
    assert_eq!(owned.len(), 1, "actual owned Run exists uniquely");
    let closed = &owned[0]["volume_acquisition_closed_at"];
    assert!(
        closed.as_str().is_some(),
        "actual acquisition closed durably"
    );
    cleanup_event(before, after, &run_id, lease_count);
    let ids = application_events(pool, before, after, run, owned[0]).await;
    product_events(before, after, closed, ids);
}

async fn application_events(
    pool: &PgPool,
    before: &Value,
    after: &Value,
    run: runtime_types::RunId,
    owned: &Value,
) -> std::collections::BTreeSet<uuid::Uuid> {
    let run_id = serde_json::json!(run);
    let closed = &owned["volume_acquisition_closed_at"];
    let (project, repository, safe_state): (uuid::Uuid, Option<uuid::Uuid>, String) =
        sqlx::query_as("SELECT i.project_id,q.repository_id,normalize_application_event_state(r.state) FROM runs r JOIN agent_instances i ON i.id=r.instance_id LEFT JOIN run_requests q ON q.run_id=r.id WHERE r.id=$1")
            .bind(run.as_uuid()).fetch_one(pool).await.unwrap();
    let application = appended(before, after, "application_events", 4);
    let occurrence = &application
        .iter()
        .find(|row| row["change_kind"] == "updated")
        .expect("acquisition UPDATE emits an application event")["occurrence_id"];
    uuid::Uuid::parse_str(occurrence.as_str().unwrap()).unwrap();
    assert!(
        !before["application_events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| &row["occurrence_id"] == occurrence)
    );
    let mut ids = std::collections::BTreeSet::new();
    for (kind, scope, change, safe, increment) in event_scopes(&run_id, owned, project, &safe_state)
    {
        let matching: Vec<_> = application
            .iter()
            .filter(|row| {
                row["scope_kind"] == kind
                    && row["scope_id"] == scope
                    && row["change_kind"] == change
            })
            .collect();
        assert_eq!(matching.len(), 1, "exact Run trigger scope {kind}");
        let row = matching[0];
        let id = uuid::Uuid::parse_str(row["id"].as_str().unwrap()).unwrap();
        assert!(ids.insert(id), "four distinct product event IDs");
        let causal_occurrence = if change == "created" {
            let target_occurrence = &row["occurrence_id"];
            uuid::Uuid::parse_str(target_occurrence.as_str().unwrap()).unwrap();
            assert_ne!(
                target_occurrence, occurrence,
                "target INSERT is a separate occurrence"
            );
            assert!(
                !before["application_events"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|old| &old["occurrence_id"] == target_occurrence)
            );
            target_occurrence
        } else {
            occurrence
        };
        application_row(
            before,
            row,
            serde_json::json!({
                "id":id, "occurrence_id":causal_occurrence, "scope_kind":kind,"scope_id":scope,
                "cursor":row["cursor"],"aggregate_type":"run","aggregate_id":run_id,
                "aggregate_version":row["aggregate_version"],"event_type":"run.changed","schema_version":1,
                "change_kind":change,"safe_state":safe,
                "related_id_one":project,"related_id_two":repository,
                "actor_type":"system","actor_id":null,"request_id":null,
                "occurred_at":closed,"retained_until":row["retained_until"]
            }),
            increment,
        );
        let exact_retention: bool = sqlx::query_scalar("SELECT e.retained_until=e.occurred_at+s.retention_interval FROM application_events e JOIN application_event_scopes s USING(scope_kind,scope_id) WHERE e.id=$1")
            .bind(id).fetch_one(pool).await.unwrap();
        assert!(exact_retention, "actual configured scope retention");
    }
    ids
}

fn event_scopes(
    run_id: &Value,
    owned: &Value,
    project: uuid::Uuid,
    safe_state: &str,
) -> [(&'static str, Value, &'static str, Value, i64); 4] {
    [
        (
            "run",
            run_id.clone(),
            "updated",
            serde_json::json!(safe_state),
            1,
        ),
        (
            "agent_instance",
            owned["instance_id"].clone(),
            "updated",
            serde_json::json!(safe_state),
            1,
        ),
        (
            "project",
            serde_json::json!(project),
            "updated",
            serde_json::json!(safe_state),
            1,
        ),
        ("run", run_id.clone(), "created", Value::Null, 2),
    ]
}

fn application_row(before: &Value, row: &Value, mut expected: Value, increment: i64) {
    let history: Vec<_> = before["application_events"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|old| old["scope_kind"] == row["scope_kind"] && old["scope_id"] == row["scope_id"])
        .collect();
    let cursor = history
        .iter()
        .map(|old| old["cursor"].as_i64().unwrap())
        .max()
        .unwrap_or(0)
        + increment;
    let version = history
        .iter()
        .filter(|old| old["aggregate_type"] == "run" && old["aggregate_id"] == row["aggregate_id"])
        .map(|old| old["aggregate_version"].as_i64().unwrap())
        .max()
        .unwrap_or(0)
        + increment;
    expected["cursor"] = serde_json::json!(cursor);
    expected["aggregate_version"] = serde_json::json!(version);
    assert_eq!(*row, expected);
}

fn cleanup_event(before: &Value, after: &Value, run_id: &Value, lease_count: usize) {
    let run_events = appended(before, after, "run_events", 1);
    let event = &run_events[0];
    uuid::Uuid::parse_str(event["id"].as_str().unwrap()).unwrap();
    let sequence = before["run_events"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| &row["run_id"] == run_id)
        .map(|row| row["sequence"].as_i64().unwrap())
        .max()
        .unwrap_or(0)
        + 1;
    assert!(event["occurred_at"].as_str().is_some());
    assert_eq!(
        *event,
        serde_json::json!({
            "id":event["id"], "run_id":run_id, "sequence":sequence,
            "event_type":"run.cleanup_started",
            "payload":{"run_id":run_id,"generation":1,"lease_count":lease_count},
            "occurred_at":event["occurred_at"]
        })
    );
}

fn product_events(
    before: &Value,
    after: &Value,
    closed: &Value,
    mut ids: std::collections::BTreeSet<uuid::Uuid>,
) {
    let product = appended(before, after, "product_event_outbox", 4);
    for row in product {
        let id = uuid::Uuid::parse_str(row["event_id"].as_str().unwrap()).unwrap();
        assert!(
            ids.remove(&id),
            "each exact application event is enqueued once"
        );
        assert_eq!(
            row,
            serde_json::json!({
                "event_id":id,"subject":"hephaestus.product.event.v1","created_at":closed,
                "published_at":null,"dead_lettered_at":null,"attempts":0,
                "last_error":null,"terminal_reason":null
            })
        );
    }
    assert!(
        ids.is_empty(),
        "all four causal events have durable outbox rows"
    );
}

fn appended(before: &Value, after: &Value, table: &str, count: usize) -> Vec<Value> {
    let mut added = after[table].as_array().unwrap().clone();
    for old in before[table].as_array().unwrap() {
        let index = added
            .iter()
            .position(|row| row == old)
            .expect("every original complete event/outbox row is preserved exactly");
        added.remove(index);
    }
    assert_eq!(added.len(), count, "exact causal append count for {table}");
    added
}
