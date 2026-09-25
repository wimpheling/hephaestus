#![allow(unused_imports)]
use super::denial::DENIAL_PROBE_CHECKS;
use super::*;
use serde_json::Value as JsonValue;
use sqlx::PgPool;
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
use uuid::Uuid;
#[derive(Debug, sqlx::FromRow)]
struct RunDiagnostic {
    state: String,
    outcome: Option<String>,
    failure: Option<String>,
    exit_code: Option<i32>,
    exit_signal: Option<i32>,
}

// Keep the bounded diagnostic payload in one place so failures remain useful
// before the composed daemon and database are torn down.
#[allow(clippy::too_many_lines)]
pub(crate) async fn run_diagnostics(pool: &PgPool, run_id: Uuid) -> String {
    let run: Option<RunDiagnostic> = sqlx::query_as(
        "SELECT state, outcome, failure, exit_code, exit_signal
               FROM runs WHERE id = $1",
    )
    .bind(run_id)
    .fetch_optional(pool)
    .await
    .expect("read run outcome for diagnostics");
    let events: Vec<(i64, String, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT sequence, event_type,
                payload->'exit'->>'code', payload->'exit'->>'signal'
           FROM run_events
         WHERE run_id = $1 ORDER BY sequence",
    )
    .bind(run_id)
    .fetch_all(pool)
    .await
    .expect("read run events for diagnostics");
    let log_chunks: Vec<JsonValue> = sqlx::query_scalar(
        "SELECT payload->'bytes'
           FROM run_events
          WHERE run_id = $1
            AND event_type = 'vm.log'
          ORDER BY sequence",
    )
    .bind(run_id)
    .fetch_all(pool)
    .await
    .expect("read session-chat log diagnostics");
    let mut log_bytes = Vec::new();
    for chunk in log_chunks {
        let Some(values) = chunk.as_array() else {
            continue;
        };
        log_bytes.extend(
            values
                .iter()
                .filter_map(JsonValue::as_u64)
                .filter_map(|value| u8::try_from(value).ok()),
        );
    }
    let safe_agent_failures: Vec<String> = String::from_utf8_lossy(&log_bytes)
        .lines()
        .filter_map(|line| line.strip_prefix("session-chat agent failed: "))
        .filter_map(|failure| {
            let fields: Vec<&str> = failure.split_whitespace().collect();
            if fields.len() == 4 && fields[0] == "git" {
                let operation = fields[1].strip_prefix("operation=")?;
                let reason = fields[2].strip_prefix("reason=")?;
                let returncode = fields[3].strip_prefix("returncode=")?.parse::<i32>().ok()?;
                let operations = [
                    "add",
                    "clone",
                    "commit",
                    "config",
                    "diff_tree",
                    "fetch",
                    "init",
                    "ls_remote",
                    "merge",
                    "push",
                    "rebase",
                    "remote",
                    "rev_list",
                    "rev_parse",
                    "rm",
                    "show",
                    "symbolic_ref",
                    "other",
                ];
                let reasons = [
                    "auth",
                    "command_failed",
                    "credential_missing",
                    "invalidrepo",
                    "missinghelper",
                    "helper_action",
                    "helper_expected_host",
                    "helper_expected_path",
                    "helper_credential_path",
                    "helper_target",
                    "helper_authority_path",
                    "helper_authority_file",
                    "helper_authority_protection",
                    "helper_authority_decode",
                    "helper_credential_missing",
                    "helper_credential_length",
                    "helper_credential_encoding",
                    "helper_internal",
                    "nonfastforward",
                    "permission",
                    "rejected",
                    "unsafeownership",
                ];
                if operations.contains(&operation)
                    && reasons.contains(&reason)
                    && (-128..=255).contains(&returncode)
                {
                    return Some(format!(
                        "git operation={operation} reason={reason} returncode={returncode}"
                    ));
                }
                return None;
            }
            let stage = failure.trim();
            (!stage.is_empty()
                && stage
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_'))
            .then(|| stage.to_owned())
        })
        .collect();
    let denial_probe_markers: Vec<String> = String::from_utf8_lossy(&log_bytes)
        .lines()
        .filter_map(|line| {
            line.split_once("HEPH_SESSION_CHAT_DENIAL_PROBE ")?
                .1
                .strip_suffix('\r')
        })
        .filter(|marker| {
            let fields: Vec<&str> = marker.split_whitespace().collect();
            fields.len() == 2
                && fields[0].starts_with("check=")
                && DENIAL_PROBE_CHECKS
                    .iter()
                    .any(|check| fields[0] == format!("check={check}"))
                && matches!(fields[1], "status=passed" | "status=failed")
        })
        .map(str::to_owned)
        .collect();
    format!(
        "run={run_id} state={} outcome={:?} failure={:?} exit_code={:?} exit_signal={:?} event_types={events:?} safe_agent_failures={safe_agent_failures:?} denial_probe_markers={denial_probe_markers:?}",
        run.as_ref().map_or("missing", |value| value.state.as_str()),
        run.as_ref().and_then(|value| value.outcome.as_deref()),
        run.as_ref().and_then(|value| value.failure.as_deref()),
        run.as_ref().and_then(|value| value.exit_code),
        run.as_ref().and_then(|value| value.exit_signal),
    )
}

pub(crate) async fn wait_for_run_succeeded(pool: &PgPool, run_id: Uuid, timeout: Duration) {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let state: Option<String> = sqlx::query_scalar("SELECT state FROM runs WHERE id = $1")
            .bind(run_id)
            .fetch_optional(pool)
            .await
            .expect("read session-chat run state");
        let succeeded: bool = sqlx::query_scalar(
            "SELECT EXISTS(
                 SELECT 1 FROM run_events
                  WHERE run_id = $1 AND event_type = 'run.succeeded'
             )",
        )
        .bind(run_id)
        .fetch_one(pool)
        .await
        .expect("read session-chat success event");
        let failed_event: bool = sqlx::query_scalar(
            "SELECT EXISTS(
                 SELECT 1 FROM run_events
                  WHERE run_id = $1 AND event_type IN ('run.failed', 'run.cancelled')
             )",
        )
        .bind(run_id)
        .fetch_one(pool)
        .await
        .expect("read session-chat failure event");
        if succeeded {
            return;
        }
        assert!(
            !(failed_event || matches!(state.as_deref(), Some("failed" | "cancelled"))),
            "session-chat run terminated before run.succeeded: {}",
            run_diagnostics(pool, run_id).await
        );
        // Keep this diagnostic before the caller tears down the daemon and
        // database; event payloads are deliberately excluded from output.
        assert!(
            tokio::time::Instant::now() < deadline,
            "session-chat run.succeeded timeout: {}",
            run_diagnostics(pool, run_id).await
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
