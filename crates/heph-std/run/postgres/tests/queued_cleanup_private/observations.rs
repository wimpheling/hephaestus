//! Whole-row graphs plus all reachable queued zero-history tables.
use super::queued_fixture::Queued;
use serde_json::Value;
use sqlx::PgPool;
#[path = "snapshot_queries.rs"]
mod snapshot_queries;

pub async fn rows(pool: &PgPool) -> Value {
    let mut all = crate::prior_observations::rows(pool).await;
    let extra: Value = sqlx::query_scalar(
        "SELECT jsonb_build_object('run_authorization_snapshots',(SELECT coalesce(jsonb_agg(to_jsonb(t)),'[]') FROM run_authorization_snapshots t),'run_authorization_snapshot_bindings',(SELECT coalesce(jsonb_agg(to_jsonb(t)),'[]') FROM run_authorization_snapshot_bindings t),'runtime_authority_sessions',(SELECT coalesce(jsonb_agg(to_jsonb(t)),'[]') FROM runtime_authority_sessions t),'capability_audit_events',(SELECT coalesce(jsonb_agg(to_jsonb(t)),'[]') FROM capability_audit_events t),'secret_runtime_sessions',(SELECT coalesce(jsonb_agg(to_jsonb(t)),'[]') FROM secret_runtime_sessions t),'secret_runtime_mounts',(SELECT coalesce(jsonb_agg(to_jsonb(t)),'[]') FROM secret_runtime_mounts t),'secret_leases',(SELECT coalesce(jsonb_agg(to_jsonb(t)),'[]') FROM secret_leases t),'run_instance_provenance',(SELECT coalesce(jsonb_agg(to_jsonb(t)),'[]') FROM run_instance_provenance t),'run_secret_provenance',(SELECT coalesce(jsonb_agg(to_jsonb(t)),'[]') FROM run_secret_provenance t),'run_vm_plan_admissions',(SELECT coalesce(jsonb_agg(to_jsonb(t)),'[]') FROM run_vm_plan_admissions t),'run_legacy_vm_placements',(SELECT coalesce(jsonb_agg(to_jsonb(t)),'[]') FROM run_legacy_vm_placements t),'run_requests',(SELECT coalesce(jsonb_agg(to_jsonb(t)),'[]') FROM run_requests t),'run_workspaces',(SELECT coalesce(jsonb_agg(to_jsonb(t)),'[]') FROM run_workspaces t),'run_results',(SELECT coalesce(jsonb_agg(to_jsonb(t)),'[]') FROM run_results t))",
    ).fetch_one(pool).await.unwrap();
    all.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    all.as_object_mut().unwrap().extend(
        snapshot_queries::extra(pool)
            .await
            .as_object()
            .unwrap()
            .clone(),
    );
    sort(&mut all);
    all
}
pub fn sort(value: &mut Value) {
    for rows in value.as_object_mut().unwrap().values_mut() {
        rows.as_array_mut().unwrap().sort_by_key(Value::to_string);
    }
}
pub async fn unconsumed(h: &Queued) {
    assert_eq!(
        h.request.original().removal_id,
        h.closure.removal_id().as_uuid()
    );
    let single = h.fixture.rows().await;
    let all = rows(&h.fixture.pool).await;
    let owned: Vec<_> = all["runs"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["id"] == serde_json::json!(h.request.run_id()))
        .collect();
    assert_eq!(owned.len(), 1);
    assert_eq!(single["run"], *owned[0]);
    let exact: (bool, bool, i64, i64) = sqlx::query_as(
        "SELECT NOT EXISTS(SELECT 1 FROM run_invocation_consumptions WHERE run_id=$1),EXISTS(SELECT 1 FROM command_inbox WHERE command_id=$2 AND command_type='start_run' AND payload=$3 AND processed_at IS NULL),(SELECT count(*) FROM agent_instance_volume_leases WHERE run_id=$1),(SELECT count(*) FROM run_vm_plan_admissions WHERE run_id=$1)",
    ).bind(h.request.run_id().as_uuid()).bind(h.fixture.start.command_id.as_uuid())
    .bind(serde_json::to_value(&h.fixture.start).unwrap()).fetch_one(&h.fixture.pool).await.unwrap();
    assert_eq!(exact, (true, true, 0, 0));
}
pub fn held(error: &run_orchestrator::RepositoryError) {
    crate::prior_observations::held(error);
}
pub fn code(error: &sqlx::Error) -> Option<String> {
    crate::prior_observations::code(error)
}
pub async fn wait_blocker(pool: &PgPool, pid: i32, event: &str) {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let found: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM pg_stat_activity a WHERE $1=ANY(pg_blocking_pids(a.pid)) AND a.wait_event=$2)",
            ).bind(pid).bind(event).fetch_one(pool).await.unwrap();
            if found { break; }
            tokio::task::yield_now().await;
        }
    }).await.expect("actual PostgreSQL blocker observed");
}

pub fn preparation_delta(h: &Queued, before: &Value, after: &Value) {
    let id = serde_json::json!(h.request.run_id());
    let mut same = after.clone();
    let original = before["runs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == id)
        .unwrap();
    let owned = same["runs"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|r| r["id"] == id)
        .unwrap();
    assert!(original["volume_acquisition_closed_at"].is_null());
    assert!(owned["volume_acquisition_closed_at"].is_string());
    let closed = owned["volume_acquisition_closed_at"].clone();
    owned["volume_acquisition_closed_at"] = Value::Null;
    let targets = after["run_cleanup_targets"].as_array().unwrap();
    assert_eq!(targets.len(), 1);
    let target = &targets[0];
    assert_eq!(
        *target,
        serde_json::json!({"run_id":id,"instance_id":h.request.instance_id(),
        "instance_revision_id":h.fixture.start.instance_revision_id,"generation":1,
        "vm_target_kind":"planned","vm_provider_namespace":h.fixture.scope.provider_namespace(),
        "vm_host_id":h.fixture.scope.host_id(),"vm_id":h.fixture.vm.0,
        "target_digest":target["target_digest"],"lease_count":0,"created_at":closed})
    );
    for table in ["application_event_scopes", "application_aggregate_versions"] {
        for row in same[table].as_array_mut().unwrap() {
            let prior = before[table].as_array().unwrap().iter().find(|old| {
                old["scope_kind"] == row["scope_kind"]
                    && old["scope_id"] == row["scope_id"]
                    && (table == "application_event_scopes"
                        || (old["aggregate_type"] == row["aggregate_type"]
                            && old["aggregate_id"] == row["aggregate_id"]))
            });
            let prior = prior.expect("all115 event scopes/aggregate rows already exist");
            let count = after["application_events"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|event| {
                    event["scope_kind"] == row["scope_kind"]
                        && event["scope_id"] == row["scope_id"]
                        && (table == "application_event_scopes"
                            || (event["aggregate_type"] == row["aggregate_type"]
                                && event["aggregate_id"] == row["aggregate_id"]))
                        && !before["application_events"]
                            .as_array()
                            .unwrap()
                            .contains(event)
                })
                .count();
            let mut expected = prior.clone();
            if count > 0 {
                let field = if table == "application_event_scopes" {
                    "committed_cursor"
                } else {
                    "aggregate_version"
                };
                expected[field] = serde_json::json!(
                    prior[field].as_i64().unwrap() + i64::try_from(count).unwrap()
                );
                if table == "application_event_scopes" {
                    expected["updated_at"] = closed.clone();
                }
            }
            assert_eq!(*row, expected, "exact causal {table} increment");
            *row = prior.clone();
        }
        assert_eq!(
            same[table].as_array().unwrap().len(),
            before[table].as_array().unwrap().len()
        );
    }
    // Exact four application/product rows and one run event were checked by
    // prior_observations::preparation_events; replace only these proved appends.
    for table in [
        "run_cleanup_targets",
        "run_events",
        "application_events",
        "product_event_outbox",
    ] {
        same[table] = before[table].clone();
    }
    sort(&mut same);
    assert_eq!(
        same, *before,
        "no unrelated row/field changed during preparation"
    );
}
