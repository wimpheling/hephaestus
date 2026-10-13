//! Caller-owned worker transactions exercise the boundary without exporting `RunPG` internals.
use super::{cleanup, fixture::Fixture, observations, running::ControlledGuest};
use runtime_types::EventId;
use serde_json::{Value, json};
use sqlx::{Postgres, Transaction};
use std::collections::BTreeMap;
use vm_trait::{VmExit, VmInstance};

pub struct Prepared {
    pub exit: VmExit,
    guest: ControlledGuest,
    rows: Value,
    files: BTreeMap<String, String>,
}
pub async fn prepare(f: &Fixture) -> Prepared {
    let guest = ControlledGuest::new(f);
    guest.start().await.unwrap();
    let exit = guest.wait().await.unwrap();
    assert_eq!(
        exit,
        VmExit {
            code: Some(0),
            signal: None
        }
    );
    cleanup::close_after_exit(f, &guest).await;
    let rows = observations::rows(f).await;
    let run = observations::run(&rows, f);
    assert_running(&run);
    for table in [
        "invocation119_terminal_transactions",
        "run_cleanup_completions",
    ] {
        assert!(rows[table].as_array().unwrap().is_empty());
    }
    assert_eq!(rows["run_cleanup_targets"].as_array().unwrap().len(), 1);
    assert_eq!(rows["run_cleanup_receipts"].as_array().unwrap().len(), 1);
    Prepared {
        exit,
        guest,
        rows,
        files: observations::files(f),
    }
}
pub async fn configure(tx: &mut Transaction<'_, Postgres>) {
    sqlx::query("SET TRANSACTION ISOLATION LEVEL READ COMMITTED, READ WRITE")
        .execute(&mut **tx)
        .await
        .unwrap();
    let actual: (String, String, String) = sqlx::query_as(
        "SELECT current_user::text,current_setting('transaction_isolation'),current_setting('transaction_read_only')",
    ).fetch_one(&mut **tx).await.unwrap();
    assert_eq!(
        actual,
        (
            "hephaestus_worker".into(),
            "read committed".into(),
            "off".into()
        )
    );
}
pub async fn xid(tx: &mut Transaction<'_, Postgres>) -> String {
    sqlx::query_scalar("SELECT pg_current_xact_id()::text")
        .fetch_one(&mut **tx)
        .await
        .unwrap()
}
pub async fn run(tx: &mut Transaction<'_, Postgres>, f: &Fixture) -> Value {
    sqlx::query_scalar("SELECT to_jsonb(r) FROM runs r WHERE id=$1")
        .bind(f.birth.start.run_id.as_uuid())
        .fetch_one(&mut **tx)
        .await
        .unwrap()
}
pub async fn events(tx: &mut Transaction<'_, Postgres>, f: &Fixture) -> Value {
    let mut rows: Value = sqlx::query_scalar(
        "SELECT coalesce(jsonb_agg(to_jsonb(e)),'[]') FROM run_events e WHERE run_id=$1",
    )
    .bind(f.birth.start.run_id.as_uuid())
    .fetch_one(&mut **tx)
    .await
    .unwrap();
    rows.as_array_mut().unwrap().sort_by_key(Value::to_string);
    rows
}
fn assert_running(row: &Value) {
    assert_eq!(row["state"], "running");
    for field in ["outcome", "exit_code", "exit_signal", "failure"] {
        assert!(row[field].is_null(), "original Running {field}");
    }
}
pub async fn candidate(tx: &mut Transaction<'_, Postgres>, f: &Fixture, exit: &VmExit) {
    assert_eq!(
        exit,
        &VmExit {
            code: Some(0),
            signal: None
        }
    );
    let effective: String =
        sqlx::query_scalar("SELECT qualify_invocation119_terminal_result($1,$2,$3,$4,$5,$6,$7,$8)")
            .bind(f.birth.start.run_id.as_uuid())
            .bind(serde_json::to_vec(&f.birth.config.canonical_json()).unwrap())
            .bind(f.birth.config.provider_scope().namespace())
            .bind(f.birth.config.provider_scope().host_id())
            .bind(true)
            .bind(exit.code)
            .bind(exit.signal)
            .bind(None::<&str>)
            .fetch_one(&mut **tx)
            .await
            .unwrap();
    assert_eq!(
        effective, "succeeded",
        "genuine uncancelled worker qualification"
    );
    let old = run(tx, f).await;
    assert_running(&old);
    sqlx::query("SELECT assert_profiled_run_transition($1,$2)")
        .bind(f.birth.start.run_id.as_uuid())
        .bind(&effective)
        .execute(&mut **tx)
        .await
        .unwrap();
    let now = crate::qualified_invocation::fixture::pg_time();
    let changed = sqlx::query(
        "UPDATE runs
         SET state = $2,
             outcome = COALESCE($3, outcome),
             exit_code = COALESCE($4, exit_code),
             exit_signal = COALESCE($5, exit_signal),
             failure = COALESCE($6, failure),
             updated_at = $7,
             state_version = state_version + 1
         WHERE id = $1",
    )
    .bind(f.birth.start.run_id.as_uuid())
    .bind(&effective)
    .bind(Some("succeeded"))
    .bind(exit.code)
    .bind(exit.signal)
    .bind(None::<&str>)
    .bind(now)
    .execute(&mut **tx)
    .await
    .unwrap();
    assert_eq!(changed.rows_affected(), 1);
    let updated_run = run(tx, f).await;
    assert_candidate(&old, &updated_run);
    let sequence: i64 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(sequence), 0) + 1 FROM run_events WHERE run_id = $1",
    )
    .bind(f.birth.start.run_id.as_uuid())
    .fetch_one(&mut **tx)
    .await
    .unwrap();
    let id = EventId::new();
    let payload = json!({
        "run_id": f.birth.start.run_id, "state": effective, "outcome": "succeeded",
        "exit": {"code": exit.code, "signal": exit.signal}, "failure": null
    });
    sqlx::query(
        "INSERT INTO run_events
         (id, run_id, sequence, event_type, payload, occurred_at)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(id.as_uuid())
    .bind(f.birth.start.run_id.as_uuid())
    .bind(sequence)
    .bind("run.succeeded")
    .bind(&payload)
    .bind(now)
    .execute(&mut **tx)
    .await
    .unwrap();
    let event: (String, Value, i64, time::OffsetDateTime) = sqlx::query_as(
        "SELECT event_type,payload,sequence,occurred_at FROM run_events WHERE id=$1",
    )
    .bind(id.as_uuid())
    .fetch_one(&mut **tx)
    .await
    .unwrap();
    assert_eq!(event, ("run.succeeded".into(), payload, sequence, now));
    assert_eq!(run(tx, f).await, updated_run);
}
fn assert_candidate(old: &Value, new: &Value) {
    assert_eq!(new["state"], "succeeded");
    assert_eq!(new["outcome"], "succeeded");
    assert_eq!(new["exit_code"], 0);
    assert!(new["exit_signal"].is_null() && new["failure"].is_null());
    assert_eq!(
        new["state_version"].as_i64().unwrap(),
        old["state_version"].as_i64().unwrap() + 1
    );
    let mut expected = old.clone();
    for field in [
        "state",
        "outcome",
        "exit_code",
        "state_version",
        "updated_at",
    ] {
        expected[field] = new[field].clone();
    }
    assert_eq!(*new, expected, "exact successful row tuple only");
}
pub async fn cleanup_denied(tx: &mut Transaction<'_, Postgres>, f: &Fixture) {
    let error = sqlx::query(
        "UPDATE runs SET state='cleaning_up',updated_at=$2,state_version=state_version+1 WHERE id=$1",
    ).bind(f.birth.start.run_id.as_uuid())
        .bind(crate::qualified_invocation::fixture::pg_time())
        .execute(&mut **tx).await.unwrap_err();
    let database = error
        .as_database_error()
        .expect("database transaction-boundary denial");
    assert_eq!(database.code().as_deref(), Some("23000"));
    assert_eq!(
        database.message(),
        "Invocation success must commit before a later row mutation"
    );
}
impl Prepared {
    pub async fn unchanged(&self, f: &Fixture) {
        assert_eq!(
            observations::rows(f).await,
            self.rows,
            "complete global graph after awaited rollback"
        );
        assert_eq!(observations::files(f), self.files);
        assert_eq!(
            self.guest.calls(),
            ["start", "wait", "destroy", "scoped_confirmation"]
        );
        f.birth.physical.assert_unchanged();
    }
}
