//! Owner observations are assertions, never execution or result authority.
use super::fixture::Fixture;
use run_domain::RunCleanupReceipt;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

const TABLES: &[&str] = &[
    "runs",
    "agent_instance_execution_invocations",
    "run_invocation_consumptions",
    "agent_instance_revision_volume_bindings",
    "agent_instance_volume_leases",
    "agent_instance_state_volumes",
    "run_authorization_snapshots",
    "run_authorization_snapshot_bindings",
    "runtime_authority_sessions",
    "run_git_authority_snapshots",
    "runtime_git_credentials",
    "run_instance_provenance",
    "run_secret_provenance",
    "secret_runtime_sessions",
    "secret_runtime_mounts",
    "secret_leases",
    "agent_secret_bindings",
    "brokered_secret_rules",
    "brokered_secret_lease_snapshots",
    "secret_audit_events",
    "brokered_secret_audit_events",
    "run_cleanup_targets",
    "run_cleanup_target_leases",
    "run_cleanup_receipts",
    "run_cleanup_completions",
    "invocation119_terminal_transactions",
    "run_events",
    "command_inbox",
    "outbox",
    "application_events",
    "product_event_outbox",
];
pub async fn rows(f: &Fixture) -> Value {
    let mut snapshot = serde_json::Map::new();
    for table in TABLES {
        let query = match *table {
            "runs" => sqlx::query_scalar::<_, Value>(
                "SELECT coalesce(jsonb_agg(to_jsonb(t)),'[]') FROM runs t",
            ),
            "agent_instance_execution_invocations" => sqlx::query_scalar::<_, Value>(
                "SELECT coalesce(jsonb_agg(to_jsonb(t)),'[]') FROM agent_instance_execution_invocations t",
            ),
            "run_invocation_consumptions" => sqlx::query_scalar::<_, Value>(
                "SELECT coalesce(jsonb_agg(to_jsonb(t)),'[]') FROM run_invocation_consumptions t",
            ),
            "agent_instance_revision_volume_bindings" => sqlx::query_scalar::<_, Value>(
                "SELECT coalesce(jsonb_agg(to_jsonb(t)),'[]') FROM agent_instance_revision_volume_bindings t",
            ),
            "agent_instance_volume_leases" => sqlx::query_scalar::<_, Value>(
                "SELECT coalesce(jsonb_agg(to_jsonb(t)),'[]') FROM agent_instance_volume_leases t",
            ),
            "agent_instance_state_volumes" => sqlx::query_scalar::<_, Value>(
                "SELECT coalesce(jsonb_agg(to_jsonb(t)),'[]') FROM agent_instance_state_volumes t",
            ),
            "run_authorization_snapshots" => sqlx::query_scalar::<_, Value>(
                "SELECT coalesce(jsonb_agg(to_jsonb(t)),'[]') FROM run_authorization_snapshots t",
            ),
            "run_authorization_snapshot_bindings" => sqlx::query_scalar::<_, Value>(
                "SELECT coalesce(jsonb_agg(to_jsonb(t)),'[]') FROM run_authorization_snapshot_bindings t",
            ),
            "runtime_authority_sessions" => sqlx::query_scalar::<_, Value>(
                "SELECT coalesce(jsonb_agg(to_jsonb(t)),'[]') FROM runtime_authority_sessions t",
            ),
            "run_git_authority_snapshots" => sqlx::query_scalar::<_, Value>(
                "SELECT coalesce(jsonb_agg(to_jsonb(t)),'[]') FROM run_git_authority_snapshots t",
            ),
            "runtime_git_credentials" => sqlx::query_scalar::<_, Value>(
                "SELECT coalesce(jsonb_agg(to_jsonb(t)),'[]') FROM runtime_git_credentials t",
            ),
            "run_instance_provenance" => sqlx::query_scalar::<_, Value>(
                "SELECT coalesce(jsonb_agg(to_jsonb(t)),'[]') FROM run_instance_provenance t",
            ),
            "run_secret_provenance" => sqlx::query_scalar::<_, Value>(
                "SELECT coalesce(jsonb_agg(to_jsonb(t)),'[]') FROM run_secret_provenance t",
            ),
            "secret_runtime_sessions" => sqlx::query_scalar::<_, Value>(
                "SELECT coalesce(jsonb_agg(to_jsonb(t)),'[]') FROM secret_runtime_sessions t",
            ),
            "secret_runtime_mounts" => sqlx::query_scalar::<_, Value>(
                "SELECT coalesce(jsonb_agg(to_jsonb(t)),'[]') FROM secret_runtime_mounts t",
            ),
            "secret_leases" => sqlx::query_scalar::<_, Value>(
                "SELECT coalesce(jsonb_agg(to_jsonb(t)),'[]') FROM secret_leases t",
            ),
            "agent_secret_bindings"
            | "brokered_secret_rules"
            | "brokered_secret_lease_snapshots"
            | "secret_audit_events"
            | "brokered_secret_audit_events" => secret_family_query(table),
            "run_cleanup_targets" => sqlx::query_scalar::<_, Value>(
                "SELECT coalesce(jsonb_agg(to_jsonb(t)),'[]') FROM run_cleanup_targets t",
            ),
            "run_cleanup_target_leases" => sqlx::query_scalar::<_, Value>(
                "SELECT coalesce(jsonb_agg(to_jsonb(t)),'[]') FROM run_cleanup_target_leases t",
            ),
            "run_cleanup_receipts" => sqlx::query_scalar::<_, Value>(
                "SELECT coalesce(jsonb_agg(to_jsonb(t)),'[]') FROM run_cleanup_receipts t",
            ),
            "run_cleanup_completions" => sqlx::query_scalar::<_, Value>(
                "SELECT coalesce(jsonb_agg(to_jsonb(t)),'[]') FROM run_cleanup_completions t",
            ),
            "invocation119_terminal_transactions" => sqlx::query_scalar::<_, Value>(
                "SELECT coalesce(jsonb_agg(to_jsonb(t)),'[]') FROM invocation119_terminal_transactions t",
            ),
            "run_events" => sqlx::query_scalar::<_, Value>(
                "SELECT coalesce(jsonb_agg(to_jsonb(t)),'[]') FROM run_events t",
            ),
            "command_inbox" => sqlx::query_scalar::<_, Value>(
                "SELECT coalesce(jsonb_agg(to_jsonb(t)),'[]') FROM command_inbox t",
            ),
            "outbox" => sqlx::query_scalar::<_, Value>(
                "SELECT coalesce(jsonb_agg(to_jsonb(t)),'[]') FROM outbox t",
            ),
            "application_events" => sqlx::query_scalar::<_, Value>(
                "SELECT coalesce(jsonb_agg(to_jsonb(t)),'[]') FROM application_events t",
            ),
            "product_event_outbox" => sqlx::query_scalar::<_, Value>(
                "SELECT coalesce(jsonb_agg(to_jsonb(t)),'[]') FROM product_event_outbox t",
            ),
            _ => unreachable!("TABLES contains only exact frozen snapshot tables"),
        };
        let mut rows: Value = query.fetch_one(&f.birth.pool).await.unwrap();
        rows.as_array_mut().unwrap().sort_by_key(Value::to_string);
        snapshot.insert((*table).to_owned(), rows);
    }
    Value::Object(snapshot)
}
fn secret_family_query(
    table: &str,
) -> sqlx::query::QueryScalar<'static, sqlx::Postgres, Value, sqlx::postgres::PgArguments> {
    match table {
        "agent_secret_bindings" => sqlx::query_scalar::<_, Value>(
            "SELECT coalesce(jsonb_agg(to_jsonb(t)),'[]') FROM agent_secret_bindings t",
        ),
        "brokered_secret_rules" => sqlx::query_scalar::<_, Value>(
            "SELECT coalesce(jsonb_agg(to_jsonb(t)),'[]') FROM brokered_secret_rules t",
        ),
        "brokered_secret_lease_snapshots" => sqlx::query_scalar::<_, Value>(
            "SELECT coalesce(jsonb_agg(to_jsonb(t)),'[]') FROM brokered_secret_lease_snapshots t",
        ),
        "secret_audit_events" => sqlx::query_scalar::<_, Value>(
            "SELECT coalesce(jsonb_agg(to_jsonb(t)),'[]') FROM secret_audit_events t",
        ),
        "brokered_secret_audit_events" => sqlx::query_scalar::<_, Value>(
            "SELECT coalesce(jsonb_agg(to_jsonb(t)),'[]') FROM brokered_secret_audit_events t",
        ),
        _ => unreachable!("TABLES contains only exact frozen snapshot tables"),
    }
}
pub fn files(f: &Fixture) -> BTreeMap<String, String> {
    f.birth
        .physical
        .files
        .iter()
        .map(|(_, path, _, _, _)| {
            (
                path.to_str().unwrap().to_owned(),
                format!("{:x}", Sha256::digest(std::fs::read(path).unwrap())),
            )
        })
        .collect()
}
pub fn run(rows: &Value, f: &Fixture) -> Value {
    rows["runs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == json!(f.birth.start.run_id))
        .unwrap()
        .clone()
}
pub fn identity(mut row: Value) -> Value {
    for field in [
        "state",
        "state_version",
        "updated_at",
        "cancel_requested_at",
    ] {
        row.as_object_mut().unwrap().remove(field);
    }
    row
}
fn successful_row(f: &Fixture, before: &Value, after: &Value) -> Value {
    let old = run(before, f);
    let new = run(after, f);
    assert_eq!(old["state"], "running");
    assert_eq!(old["outcome"], Value::Null);
    assert_eq!(old["exit_code"], Value::Null);
    assert_eq!(new["state"], "succeeded");
    assert_eq!(new["outcome"], "succeeded");
    assert_eq!(new["exit_code"], 0);
    assert!(new["exit_signal"].is_null() && new["failure"].is_null());
    assert_eq!(
        new["state_version"].as_i64().unwrap(),
        old["state_version"].as_i64().unwrap() + 1
    );
    let mut expected = old;
    for field in [
        "state",
        "state_version",
        "outcome",
        "exit_code",
        "updated_at",
    ] {
        expected[field] = new[field].clone();
    }
    assert_eq!(
        new, expected,
        "only exact successful transition fields changed"
    );
    new
}
pub async fn success(f: &Fixture, before: &Value, after: &Value, count: usize) {
    let new = successful_row(f, before, after);
    for table in TABLES {
        if ![
            "runs",
            "invocation119_terminal_transactions",
            "run_events",
            "outbox",
            "application_events",
            "product_event_outbox",
        ]
        .contains(table)
        {
            assert_eq!(
                before[*table], after[*table],
                "unchanged historical {table}"
            );
        }
    }
    let markers = after["invocation119_terminal_transactions"]
        .as_array()
        .unwrap();
    assert_eq!(markers.len(), 1);
    assert_eq!(markers[0]["run_id"], new["id"]);
    assert_eq!(markers[0]["success_version"], new["state_version"]);
    assert_eq!(markers[0]["identity"], identity(new.clone()));
    assert!(!markers[0]["success_xid"].is_null());
    let stored: (String, i64, Vec<u8>, Vec<u8>, bool) = sqlx::query_as("SELECT marker.success_xid::text,marker.success_version,target.target_digest,receipt.target_digest,marker.success_xid<>pg_current_xact_id() FROM invocation119_terminal_transactions marker JOIN run_cleanup_targets target USING(run_id) JOIN run_cleanup_receipts receipt USING(run_id,generation) WHERE marker.run_id=$1").bind(f.birth.start.run_id.as_uuid()).fetch_one(&f.birth.pool).await.unwrap();
    assert_eq!(stored.1, new["state_version"].as_i64().unwrap());
    assert_eq!(stored.2, stored.3);
    assert!(
        stored.4,
        "success marker belongs to the already committed transaction"
    );
    assert!(
        after["run_cleanup_completions"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let leases = after["agent_instance_volume_leases"].as_array().unwrap();
    assert_eq!(leases.len(), count);
    for lease in leases {
        assert_eq!(lease["run_id"], new["id"]);
        assert_eq!(lease["state"], "active");
        assert!(lease["attached_at"].is_string());
        assert!(lease["released_at"].is_null());
    }
    let events = after["run_events"].as_array().unwrap();
    let added: Vec<_> = events
        .iter()
        .filter(|e| !before["run_events"].as_array().unwrap().contains(e))
        .collect();
    assert_eq!(added.len(), 1);
    assert_eq!(added[0]["event_type"], "run.succeeded");
    assert_eq!(added[0]["payload"]["exit"], json!({"code":0,"signal":null}));
    assert_eq!(added[0]["payload"]["state"], "succeeded");
    assert_eq!(added[0]["payload"]["outcome"], "succeeded");
    f.birth.physical.assert_unchanged();
}
pub async fn completed(f: &Fixture, receipt: &RunCleanupReceipt) {
    let rows = rows(f).await;
    let target = &rows["run_cleanup_targets"][0];
    let stored = &rows["run_cleanup_receipts"][0];
    let completion = &rows["run_cleanup_completions"][0];
    assert_eq!(target["target_digest"], stored["target_digest"]);
    assert_eq!(target["target_digest"], completion["target_digest"]);
    assert_eq!(target["generation"], json!(receipt.target().generation()));
    assert_eq!(completion["generation"], target["generation"]);
    assert_eq!(completion["run_id"], json!(f.birth.start.run_id));
    for lease in rows["agent_instance_volume_leases"].as_array().unwrap() {
        assert_eq!(lease["state"], "released");
        assert!(lease["released_at"].is_string());
    }
    for fence in receipt.target().leases() {
        let volume = rows["agent_instance_state_volumes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["id"] == json!(fence.volume_id()))
            .unwrap();
        assert_eq!(volume["state"], "ready");
        assert_eq!(volume["lease_generation"], json!(fence.fencing_token()));
    }
    assert_eq!(
        rows["invocation119_terminal_transactions"][0]["identity"],
        identity(run(&rows, f))
    );
}
