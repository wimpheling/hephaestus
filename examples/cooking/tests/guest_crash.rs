//! Joined deterministic guest-crash assertions.
//!
//! The transformed release is built and launched by the golden fixture. This
//! module only drives real ingress and reads committed run/lease history; it
//! never fabricates lifecycle rows or mutates PostgreSQL/SQLite state.

use super::GatewayGoldenFixture;
use super::cooking_builds::PreparedCookingInstance;
use serde_json::Value;
use sqlx::PgPool;
use std::{path::Path, process::Command, time::Duration};
use uuid::Uuid;

type CrashAttempt = (i32, Uuid, String, Option<String>, Option<i32>);

const PROBE_BEFORE: &str = "cooking probe before: surfaces=release,control,state,work,repo,secret-mount,runtime-credential,authority,proc-env,proc-argv;";
const PROBE_AFTER: &str = "cooking probe after: surfaces=release,control,state,work,repo,secret-mount,runtime-credential,authority,proc-env,proc-argv;";

/// Sends updates 51..55 through the configured crash mailbox and proves that
/// each signal-9 attempt is recovered exactly once. The second attempt is
/// selected by the existing durable delivery key; no duplicate ingress is
/// generated to make recovery look successful.
pub async fn exercise(
    pool: &PgPool,
    gateway: &GatewayGoldenFixture,
    instance: &PreparedCookingInstance,
) {
    let public =
        std::env::var("HEPHAESTUS_CADDY_TEST_PUBLIC_URL").expect("joined cooking public URL");
    let url = format!("{public}/gateway/cooking/telegram");
    let client = reqwest::Client::new();
    for (offset, text) in [
        "before-commit",
        "after-commit",
        "model-persist",
        "relay-persist",
        "proposal-ready",
    ]
    .into_iter()
    .enumerate()
    {
        let update_id = 51 + u64::try_from(offset).expect("bounded update offset");
        let response = super::cooking::send_update(&client, &url, update_id, 1001, text).await;
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        response
            .bytes()
            .await
            .expect("cooking ingress acknowledgement");
        let run = super::cooking::wait_for_event_run(pool, gateway, update_id).await;
        let (accepted, duplicate): (i64, i64) = sqlx::query_as(
            "SELECT count(*) FILTER (WHERE outcome = 'accepted'),
                    count(*) FILTER (WHERE outcome = 'duplicate')
               FROM gateway_mailbox_publications
              WHERE mailbox_id = $1 AND deduplication_key = $2",
        )
        .bind(gateway.mailbox_id.as_uuid())
        .bind(format!("telegram-update-{update_id}"))
        .fetch_one(pool)
        .await
        .expect("crash publication census");
        assert_eq!(accepted, 1, "one ingress event must be accepted");
        assert_eq!(duplicate, 0, "crash recovery must not replay ingress");
        let recovered_run = assert_crash_recovery(pool, run.event_id, instance.instance_id).await;
        assert_eq!(recovered_run, run.run_id.as_uuid());
    }
}

async fn assert_crash_recovery(pool: &PgPool, event_id: Uuid, instance_id: Uuid) -> Uuid {
    let logical_attempts: i32 = sqlx::query_scalar(
        "SELECT logical_attempt_count FROM mailbox_deliveries WHERE event_id = $1",
    )
    .bind(event_id)
    .fetch_one(pool)
    .await
    .expect("guest crash logical attempt count");
    assert_eq!(logical_attempts, 2, "one durable retry per crash event");
    let attempts: Vec<CrashAttempt> = sqlx::query_as(
        "SELECT attempt.attempt_number, run.id, run.state, run.outcome,
                run.exit_signal
           FROM mailbox_delivery_attempts attempt
           JOIN runs run ON run.id = attempt.run_id
          WHERE attempt.event_id = $1
          ORDER BY attempt.attempt_number",
    )
    .bind(event_id)
    .fetch_all(pool)
    .await
    .expect("guest crash attempt history");
    assert_eq!(
        attempts.len(),
        2,
        "each crash event has one recovered retry"
    );
    assert_eq!(attempts[0].2, "cleaned_up");
    assert_eq!(attempts[0].3.as_deref(), Some("failed"));
    assert_eq!(attempts[0].4, Some(9), "first attempt was SIGKILL");
    assert_eq!(attempts[1].2, "cleaned_up");
    assert_eq!(attempts[1].3.as_deref(), Some("succeeded"));
    let first_instance: Uuid = sqlx::query_scalar("SELECT instance_id FROM runs WHERE id = $1")
        .bind(attempts[0].1)
        .fetch_one(pool)
        .await
        .expect("first crash run instance");
    let second_instance: Uuid = sqlx::query_scalar("SELECT instance_id FROM runs WHERE id = $1")
        .bind(attempts[1].1)
        .fetch_one(pool)
        .await
        .expect("recovered crash run instance");
    assert_eq!(first_instance, instance_id);
    assert_eq!(second_instance, instance_id);
    let leases: Vec<(time::OffsetDateTime, Option<time::OffsetDateTime>)> = sqlx::query_as(
        "SELECT acquired_at, released_at
           FROM agent_instance_volume_leases
          WHERE run_id = ANY($1)
          ORDER BY acquired_at",
    )
    .bind(vec![attempts[0].1, attempts[1].1])
    .fetch_all(pool)
    .await
    .expect("crash lease history");
    assert_eq!(leases.len(), 2, "both attempts retain state lease history");
    assert!(
        leases[0].1.is_some_and(|released| released <= leases[1].0),
        "recovered attempt must acquire its state volume after SIGKILL cleanup"
    );
    assert!(leases[1].1.is_some(), "recovered attempt must clean up");
    assert_probe_evidence(pool, attempts[0].1, attempts[1].1).await;
    attempts[1].1
}

/// The runtime probe writes only compact, flushed stderr summaries. The host
/// checks the persisted VM log stream per attempt so a successful retry cannot
/// hide a missing first-attempt scan (or vice versa).
async fn assert_probe_evidence(pool: &PgPool, first_run: Uuid, retry_run: Uuid) {
    let first = probe_log(pool, first_run).await;
    let retry = probe_log(pool, retry_run).await;
    assert_probe_summary(&first, PROBE_BEFORE);
    assert_probe_summary(&retry, PROBE_BEFORE);
    assert_probe_summary(&retry, PROBE_AFTER);
    assert_eq!(
        first.matches(PROBE_BEFORE).count(),
        1,
        "crashed attempt probe evidence"
    );
    assert_eq!(
        first.matches(PROBE_AFTER).count(),
        0,
        "crashed attempt has no after evidence"
    );
    assert_eq!(
        retry.matches(PROBE_BEFORE).count(),
        1,
        "retry before probe evidence"
    );
    assert_eq!(
        retry.matches(PROBE_AFTER).count(),
        1,
        "retry after probe evidence"
    );
}

fn assert_probe_summary(log: &str, marker: &str) {
    let start = log.find(marker).expect("probe summary marker");
    let summary = &log[start + marker.len()..];
    let (files, bytes) = summary
        .split_once(";bytes=")
        .expect("probe summary byte count");
    let files = files
        .strip_prefix("files=")
        .expect("probe summary file count")
        .parse::<u64>()
        .expect("numeric probe file count");
    let bytes = bytes
        .split_whitespace()
        .next()
        .expect("probe summary bytes")
        .parse::<u64>()
        .expect("numeric probe byte count");
    assert!(files > 0, "probe must inspect at least one file");
    assert!(bytes > 0, "probe must inspect nonempty surfaces");
}

async fn probe_log(pool: &PgPool, run_id: Uuid) -> String {
    let payloads: Vec<Value> = sqlx::query_scalar(
        "SELECT payload FROM run_events
          WHERE run_id = $1 AND event_type = 'vm.log'
          ORDER BY sequence",
    )
    .bind(run_id)
    .fetch_all(pool)
    .await
    .expect("guest probe VM logs");
    payloads
        .iter()
        .filter_map(|payload| payload.get("bytes").and_then(Value::as_array))
        .flatten()
        .filter_map(Value::as_u64)
        .filter_map(|byte| u8::try_from(byte).ok())
        .map(char::from)
        .collect()
}

/// Bounded wait used by golden after all five crash events. It prevents
/// teardown from dropping the relay ledger before the exact physical census.
pub async fn wait_for_upstream(upstream: super::BrokeredTlsUpstream) {
    tokio::time::timeout(Duration::from_secs(90), upstream.wait_complete())
        .await
        .expect("guest crash upstream timeout")
        .expect("guest crash upstream task");
}

/// Reads the detached `/cooking.sqlite3` snapshot through `SQLite` and checks
/// the exact five crash recipes. Main/WAL/SHM extraction is owned by the
/// existing state-volume inspection helper; this function intentionally does
/// no raw-byte or SQL mutation shortcut.
pub fn assert_sqlite_snapshot(path: &Path) {
    let script = r#"
import sqlite3, sys
db = sqlite3.connect("file:" + sys.argv[1] + "?mode=ro", uri=True)
assert db.execute("SELECT version FROM schema_meta").fetchone()[0] == 1
columns = {row[1] for row in db.execute("PRAGMA table_info(recipes)")}
assert {"context", "rendered_content"} <= columns
rows = db.execute(
    "SELECT recipe_id, publication_outcome, model_response, relay_outcome, context FROM recipes"
).fetchall()
ids = {row[0] for row in rows}
assert ids == {"recipe-51", "recipe-52", "recipe-53", "recipe-54", "recipe-55"}, ids
assert all(row[1] == "proposal_ready" and row[2] and row[3] and row[4] for row in rows)
assert db.execute("SELECT count(*) FROM processed_updates").fetchone()[0] == 5
assert db.execute("SELECT count(*) FROM recipes WHERE context = ''").fetchone()[0] == 0
assert db.execute("SELECT count(*) FROM processed_updates WHERE disposition != 'completed'").fetchone()[0] == 0
db.close()
#"#;
    let output = Command::new("python3")
        .arg("-c")
        .arg(script)
        .arg(path)
        .output()
        .expect("run guest crash SQLite inspection");
    assert!(
        output.status.success(),
        "guest crash SQLite schema/recipe inspection failed"
    );
}

/// Extracts `/cooking.sqlite3` and optional WAL/SHM sidecars from a detached
/// state-volume image before running the exact SQL inspection.
pub fn assert_sqlite_disk(disk: &Path) {
    assert!(disk.is_file(), "guest crash state-volume image is required");
    let debugfs = Command::new("debugfs")
        .arg("-V")
        .output()
        .expect("debugfs is required for guest crash inspection");
    assert!(debugfs.status.success(), "debugfs prerequisite failed");
    let temporary = tempfile::tempdir().expect("guest crash SQLite snapshot directory");
    let snapshot = temporary.path().join("cooking.sqlite3");
    for (guest, destination, required) in [
        ("/cooking.sqlite3", snapshot.clone(), true),
        (
            "/cooking.sqlite3-wal",
            snapshot.with_extension("sqlite3-wal"),
            false,
        ),
        (
            "/cooking.sqlite3-shm",
            snapshot.with_extension("sqlite3-shm"),
            false,
        ),
    ] {
        let destination_string = destination.to_str().expect("snapshot path UTF-8");
        let output = Command::new("debugfs")
            .args(["-R", &format!("dump {guest} {destination_string}")])
            .arg(disk)
            .output()
            .expect("extract guest crash SQLite file");
        if required {
            assert!(output.status.success(), "required SQLite extraction failed");
            assert!(destination.is_file(), "debugfs did not extract SQLite file");
        }
    }
    super::cooking_confinement::assert_state_snapshot_has_no_credentials(temporary.path());
    assert_sqlite_snapshot(&snapshot);
}
