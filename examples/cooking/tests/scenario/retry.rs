// Reuse the scenario facade imports so each phase preserves the production fixture context.
#[allow(unused_imports)]
use super::*;
pub(crate) async fn deliver_request(
    pool: &sqlx::PgPool,
    gateway: &GatewayGoldenFixture,
    client: &reqwest::Client,
    url: &str,
    update_id: u64,
    provider_id: u64,
    text: &str,
) -> CookingRun {
    let accepted = send_update(client, url, update_id, provider_id, text).await;
    assert_eq!(accepted.status(), reqwest::StatusCode::OK);
    accepted
        .bytes()
        .await
        .expect("cooking acknowledgement body");
    wait_for_event_run(pool, gateway, update_id).await
}

pub(crate) async fn wait_for_retried_attempt_completion(
    pool: &sqlx::PgPool,
    mailbox_id: uuid::Uuid,
    event_id: uuid::Uuid,
    run_id: uuid::Uuid,
) {
    // `wait_for_event_run` observes the run cleanup transaction. The mailbox
    // completion observer settles its exact attempt in a following transaction,
    // so this assertion may briefly see the returned successful run's attempt
    // as leased or running. Wait only for that known attempt to reach completed.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let state: Option<String> = sqlx::query_scalar(
            "SELECT state FROM mailbox_delivery_attempts
              WHERE event_id = $1 AND attempt_number = 2 AND run_id = $2",
        )
        .bind(event_id)
        .bind(run_id)
        .fetch_optional(pool)
        .await
        .expect("durable retry attempt projection");
        match state.as_deref() {
            Some("completed") => return,
            Some("leased" | "running") => {}
            Some(other) => {
                write_retry_failure_evidence(pool, mailbox_id, event_id, run_id, Some(other)).await;
                panic!("retry attempt entered unexpected state: {other}");
            }
            None => {
                write_retry_failure_evidence(pool, mailbox_id, event_id, run_id, None).await;
                panic!("successful retry attempt identity is absent");
            }
        }
        let within_deadline = tokio::time::Instant::now() < deadline;
        if !within_deadline {
            write_retry_failure_evidence(pool, mailbox_id, event_id, run_id, Some("timeout")).await;
        }
        assert!(
            within_deadline,
            "successful retry attempt completion projection timed out"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

pub(crate) async fn write_retry_failure_evidence(
    pool: &sqlx::PgPool,
    mailbox_id: uuid::Uuid,
    event_id: uuid::Uuid,
    run_id: uuid::Uuid,
    observed_state: Option<&str>,
) {
    // Flush the exact mailbox lineage before the panic is propagated.  The
    // periodic snapshot can predate the retry's terminal transition, so this
    // final, event-scoped sample captures the latest state before the panic.
    write_cooking_lineage_snapshot(pool, mailbox_id, Some(event_id)).await;

    let lookup = tokio::time::timeout(
        Duration::from_secs(2),
        sqlx::query_as::<_, RetryAttemptRow>(
            "SELECT attempt.id, attempt.attempt_number, attempt.state,
                    run.state, run.outcome, run.exit_code, run.exit_signal
               FROM mailbox_delivery_attempts attempt
               JOIN runs run ON run.id = attempt.run_id
              WHERE attempt.event_id = $1 AND attempt.run_id = $2
              ORDER BY attempt.attempt_number DESC LIMIT 1",
        )
        .bind(event_id)
        .bind(run_id)
        .fetch_optional(pool),
    )
    .await;
    let evidence = match lookup {
        Ok(Ok(Some(row))) => RetryAttemptEvidence::from_row(&row, observed_state, "ok"),
        Ok(Ok(None)) => RetryAttemptEvidence::missing(observed_state),
        Ok(Err(_)) => RetryAttemptEvidence::unavailable(observed_state, "query-failed"),
        Err(_) => RetryAttemptEvidence::unavailable(observed_state, "query-timeout"),
    };
    let attempt_id = evidence
        .attempt_id
        .map_or_else(|| String::from("unknown"), |value| value.to_string());
    let attempt_number = evidence
        .attempt_number
        .map_or_else(|| String::from("unknown"), |value| value.to_string());
    let exit_code = evidence
        .exit_code
        .map_or_else(|| String::from("none"), |value| value.to_string());
    let exit_signal = evidence
        .exit_signal
        .map_or_else(|| String::from("none"), |value| value.to_string());
    let classification = match observed_state {
        Some("timeout") => "retry-completion-timeout",
        _ => match evidence.attempt_state {
            "failed" => "retry-terminal-failed",
            "uncertain" => "retry-terminal-uncertain",
            _ => "retry-terminal-unresolved",
        },
    };
    let marker = format!(
        concat!(
            "HEPH_COOKING_RETRY event=terminal classification={classification} ",
            "lookup_status={lookup_status} event_id={event_id} attempt_id={attempt_id} ",
            "attempt_number={attempt_number} run_id={run_id} attempt_state={attempt_state} ",
            "run_state={run_state} run_outcome={run_outcome} exit_code={exit_code} ",
            "exit_signal={exit_signal}"
        ),
        classification = classification,
        lookup_status = evidence.lookup_status,
        event_id = event_id,
        attempt_id = attempt_id,
        attempt_number = attempt_number,
        run_id = run_id,
        attempt_state = evidence.attempt_state,
        run_state = evidence.run_state,
        run_outcome = evidence.run_outcome,
        exit_code = exit_code,
        exit_signal = exit_signal,
    );
    let mut stderr = std::io::stderr();
    let _ = writeln!(stderr, "{marker}");
    let _ = stderr.flush();
}

pub(crate) type RetryAttemptRow = (
    uuid::Uuid,
    i32,
    String,
    String,
    Option<String>,
    Option<i32>,
    Option<i32>,
);

pub(crate) struct RetryAttemptEvidence {
    attempt_id: Option<uuid::Uuid>,
    attempt_number: Option<i32>,
    attempt_state: &'static str,
    run_state: &'static str,
    run_outcome: &'static str,
    exit_code: Option<i32>,
    exit_signal: Option<i32>,
    lookup_status: &'static str,
}

impl RetryAttemptEvidence {
    fn from_row(
        row: &RetryAttemptRow,
        observed_state: Option<&str>,
        lookup_status: &'static str,
    ) -> Self {
        Self {
            attempt_id: Some(row.0),
            attempt_number: Some(row.1),
            attempt_state: safe_retry_attempt_state(Some(row.2.as_str()), observed_state),
            run_state: safe_retry_run_state(Some(row.3.as_str())),
            run_outcome: safe_retry_run_outcome(row.4.as_deref()),
            exit_code: row.5,
            exit_signal: row.6,
            lookup_status,
        }
    }

    fn missing(observed_state: Option<&str>) -> Self {
        Self {
            attempt_id: None,
            attempt_number: None,
            attempt_state: safe_retry_attempt_state(None, observed_state),
            run_state: "unknown",
            run_outcome: "none",
            exit_code: None,
            exit_signal: None,
            lookup_status: "missing",
        }
    }

    fn unavailable(observed_state: Option<&str>, lookup_status: &'static str) -> Self {
        Self {
            attempt_id: None,
            attempt_number: None,
            attempt_state: safe_retry_attempt_state(None, observed_state),
            run_state: "unknown",
            run_outcome: "none",
            exit_code: None,
            exit_signal: None,
            lookup_status,
        }
    }
}

pub(crate) fn safe_retry_attempt_state(
    row_state: Option<&str>,
    observed_state: Option<&str>,
) -> &'static str {
    match row_state.or(observed_state) {
        Some("leased") => "leased",
        Some("running") => "running",
        Some("completed") => "completed",
        Some("failed") => "failed",
        Some("uncertain") => "uncertain",
        _ => "unknown",
    }
}

pub(crate) fn safe_retry_run_state(value: Option<&str>) -> &'static str {
    match value {
        Some("queued") => "queued",
        Some("leasing_volume") => "leasing_volume",
        Some("provisioning") => "provisioning",
        Some("starting") => "starting",
        Some("running") => "running",
        Some("succeeded") => "succeeded",
        Some("failed") => "failed",
        Some("cancelled") => "cancelled",
        Some("cleaning_up") => "cleaning_up",
        Some("cleaned_up") => "cleaned_up",
        _ => "unknown",
    }
}

pub(crate) fn safe_retry_run_outcome(value: Option<&str>) -> &'static str {
    match value {
        None => "none",
        Some("succeeded") => "succeeded",
        Some("failed") => "failed",
        Some("cancelled") => "cancelled",
        _ => "unknown",
    }
}

pub(crate) async fn assert_retried_delivery(
    pool: &sqlx::PgPool,
    mailbox_id: uuid::Uuid,
    successful_run: CookingRun,
) {
    wait_for_retried_attempt_completion(
        pool,
        mailbox_id,
        successful_run.event_id,
        successful_run.run_id.as_uuid(),
    )
    .await;

    let logical_attempts: i32 = sqlx::query_scalar(
        "SELECT logical_attempt_count FROM mailbox_deliveries WHERE event_id = $1",
    )
    .bind(successful_run.event_id)
    .fetch_one(pool)
    .await
    .expect("durable outbound fault retry count");
    assert_eq!(
        logical_attempts, 2,
        "fault recovery creates one explicit retry"
    );

    let attempts: Vec<(i32, uuid::Uuid, String, String, Option<String>)> = sqlx::query_as(
        "SELECT attempt.attempt_number, attempt.run_id, attempt.state,
                    run.state, run.outcome
               FROM mailbox_delivery_attempts attempt
               JOIN runs run ON run.id = attempt.run_id
              WHERE attempt.event_id = $1
              ORDER BY attempt.attempt_number",
    )
    .bind(successful_run.event_id)
    .fetch_all(pool)
    .await
    .expect("durable outbound fault retry attempts");
    assert_eq!(attempts.len(), 2, "fault recovery retains both attempts");
    assert_eq!(attempts[0].0, 1, "first fault attempt is numbered one");
    assert_eq!(attempts[1].0, 2, "retry attempt is numbered two");
    assert_ne!(
        attempts[0].1, attempts[1].1,
        "fault recovery creates distinct runs"
    );
    assert!(
        matches!(attempts[0].2.as_str(), "failed" | "uncertain"),
        "first fault attempt remains inspectable with an explicit terminal state: {}",
        attempts[0].2
    );
    assert_eq!(attempts[0].3, "cleaned_up");
    assert_eq!(
        attempts[0].4.as_deref(),
        Some("failed"),
        "first fault run is durably failed"
    );
    assert_eq!(
        attempts[1].2, "completed",
        "only the retry completes the attempt"
    );
    assert_eq!(attempts[1].3, "cleaned_up");
    assert_eq!(
        attempts[1].4.as_deref(),
        Some("succeeded"),
        "only the retry completes the run"
    );
    assert_eq!(
        attempts[1].1,
        successful_run.run_id.as_uuid(),
        "returned run is the durable successful retry"
    );
}

// This is an acceptance scenario whose branches each assert a separate
// boundary; keeping the sequence visible makes failures directly actionable.
