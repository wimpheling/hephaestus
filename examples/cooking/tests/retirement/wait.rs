use super::RetryObservation;
use sqlx::PgPool;
use std::time::Duration;
use uuid::Uuid;

pub(crate) async fn wait_for_control_terminal(
    pool: &PgPool,
    id: Uuid,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    Ok(tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let state: String =
                sqlx::query_scalar("SELECT state FROM control_requests WHERE id = $1")
                    .bind(id)
                    .fetch_one(pool)
                    .await?;
            if state == "failed" || state == "completed" {
                return Ok::<String, sqlx::Error>(state);
            }
            assert!(state == "pending" || state == "processing");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await??)
}

pub(crate) async fn wait_for_failed_retry(
    pool: &PgPool,
    source_run_id: Uuid,
    existing_retry_ids: &[Uuid],
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let row: Option<RetryObservation> = sqlx::query_as(
                "SELECT run.id, run.state, run.outcome, run.failure, run.vm_id
                   FROM runs AS run
                   JOIN run_requests AS request ON request.run_id = run.id
                  WHERE request.retry_of_run_id = $1 AND run.id <> ALL($2)
                  ORDER BY run.created_at DESC, run.id DESC
                  LIMIT 1",
            )
            .bind(source_run_id)
            .bind(existing_retry_ids)
            .fetch_optional(pool)
            .await?;
            if let Some(retry) = row {
                if retry.state == "cleaned_up" {
                    assert_eq!(retry.outcome.as_deref(), Some("failed"));
                    assert!(
                        retry
                            .failure
                            .as_deref()
                            .is_some_and(|value| {
                                value
                                    == "invalid VM specification field \"instance_revision\": the exact revision or attachment is not runnable"
                            })
                    );
                    // The orchestrator may reserve the run ID as `vm_id` before
                    // validating the immutable spec; that reservation is not
                    // evidence that the provider created a VM.
                    let reserved_vm_id = retry.id.to_string();
                    assert!(
                        retry
                            .vm_id
                            .as_deref()
                            .is_none_or(|vm_id| vm_id == reserved_vm_id),
                        "denied retry has an unexpected VM identifier"
                    );
                    let lifecycle_events: i64 = sqlx::query_scalar(
                        "SELECT count(*)
                           FROM run_events
                          WHERE run_id = $1
                            AND event_type IN (
                                'vm.started', 'vm.ready', 'vm.exited',
                                'run.starting', 'run.running'
                            )",
                    )
                    .bind(retry.id)
                    .fetch_one(pool)
                    .await?;
                    assert_eq!(lifecycle_events, 0);
                    assert_ne!(retry.id, source_run_id);
                    return Ok::<(), sqlx::Error>(());
                }
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await??;
    Ok(())
}
