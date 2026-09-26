// Reuse the scenario facade imports so each phase preserves the production fixture context.
#[allow(unused_imports)]
use super::*;
pub(crate) fn cooking_lineage_row_json(
    row: CookingLineageRow,
    mailbox_id: uuid::Uuid,
    sampled_at: &str,
) -> String {
    let (
        event_id,
        attempt_id,
        attempt_number,
        attempt_run_id,
        attempt_state,
        attempt_created_at,
        attempt_completed_at,
        run_state,
        run_outcome,
        run_exit_code,
        run_exit_signal,
        disposition,
        next_eligible_at,
        terminal_at,
        run_created_at,
        run_updated_at,
    ) = row;
    serde_json::json!({
        "sampled_at": sampled_at,
        "mailbox_id": mailbox_id,
        "event_id": event_id,
        "attempt_id": attempt_id,
        "attempt_number": attempt_number,
        "attempt_run_id": attempt_run_id,
        "attempt_state": attempt_state,
        "attempt_created_at": attempt_created_at.to_string(),
        "attempt_completed_at": attempt_completed_at.map(|value| value.to_string()),
        "run_state": run_state,
        "run_outcome": run_outcome,
        "exit_code": run_exit_code,
        "exit_signal": run_exit_signal,
        "disposition": disposition,
        "next_eligible_at": next_eligible_at.map(|value| value.to_string()),
        "terminal_at": terminal_at.map(|value| value.to_string()),
        "run_created_at": run_created_at.to_string(),
        "run_updated_at": run_updated_at.to_string(),
    })
    .to_string()
}

/// Periodically exports only current Cooking delivery/run lifecycle metadata.
/// The file is atomically replaced so a killed test leaves the last complete
/// sample for the external diagnostics collector. Payloads, failures, and
/// credentials are intentionally outside this query.
pub(crate) async fn write_cooking_lineage_snapshot(
    pool: &sqlx::PgPool,
    mailbox_id: uuid::Uuid,
    event_id: Option<uuid::Uuid>,
) {
    let Some(directory) = env::var_os("HEPHAESTUS_COOKING_DIAGNOSTICS_DIR") else {
        return;
    };
    let directory = PathBuf::from(directory);
    if !directory.is_absolute() || directory.is_symlink() || !directory.is_dir() {
        return;
    }
    let rows = sqlx::query_as::<_, CookingLineageRow>(
        "SELECT event.id, attempt.id, attempt.attempt_number, attempt.run_id,
                attempt.state, attempt.created_at, attempt.completed_at,
                run.state, run.outcome, run.exit_code, run.exit_signal,
                delivery.disposition,
                delivery.next_eligible_at, delivery.terminal_at,
                run.created_at, run.updated_at
           FROM mailbox_events event
           JOIN mailbox_delivery_attempts attempt ON attempt.event_id = event.id
           JOIN mailbox_deliveries delivery ON delivery.event_id = event.id
           JOIN runs run ON run.id = attempt.run_id
          WHERE event.mailbox_id = $1
            AND ($2::uuid IS NULL OR event.id = $2)
          ORDER BY event.id, attempt.attempt_number, attempt.id
          LIMIT 5000",
    )
    .bind(mailbox_id)
    .bind(event_id)
    .fetch_all(pool);
    let rows = match tokio::time::timeout(Duration::from_secs(2), rows).await {
        Ok(Ok(rows)) => rows,
        Ok(Err(_)) => {
            write_cooking_lineage_status(&directory, mailbox_id, event_id, "query_failed", 0);
            return;
        }
        Err(_) => {
            write_cooking_lineage_status(&directory, mailbox_id, event_id, "query_timeout", 0);
            return;
        }
    };
    let sampled_at = time::OffsetDateTime::now_utc().to_string();
    let lines = rows
        .into_iter()
        .map(|row| cooking_lineage_row_json(row, mailbox_id, &sampled_at))
        .collect::<Vec<_>>();
    let contents = if lines.is_empty() {
        String::new()
    } else {
        format!("{}\n", lines.join("\n"))
    };
    if !write_cooking_diagnostic_file(&directory, "cooking-lineage.jsonl", contents.as_bytes()) {
        write_cooking_lineage_status(
            &directory,
            mailbox_id,
            event_id,
            "write_failed",
            lines.len(),
        );
        return;
    }
    write_cooking_lineage_status(&directory, mailbox_id, event_id, "ok", lines.len());
}

pub(crate) fn write_cooking_diagnostic_file(directory: &Path, name: &str, contents: &[u8]) -> bool {
    let destination = directory.join(name);
    let temporary = directory.join(format!(".{name}.tmp.{}", uuid::Uuid::new_v4()));
    fs::write(&temporary, contents)
        .and_then(|()| fs::rename(temporary, destination))
        .is_ok()
}

pub(crate) fn write_cooking_lineage_status(
    directory: &Path,
    mailbox_id: uuid::Uuid,
    event_id: Option<uuid::Uuid>,
    status: &str,
    rows: usize,
) {
    let value = serde_json::json!({
        "schema": 1,
        "status": status,
        "sampled_at": time::OffsetDateTime::now_utc().to_string(),
        "mailbox_id": mailbox_id,
        "event_id": event_id,
        "rows": rows,
    });
    let _ = write_cooking_diagnostic_file(
        directory,
        "cooking-lineage-status.json",
        format!("{value}\n").as_bytes(),
    );
}
