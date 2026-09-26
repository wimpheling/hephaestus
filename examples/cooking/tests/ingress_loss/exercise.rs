use super::super::cooking::CookingRun;
use super::{IngressLossContext, IngressLossProxy, caddy_host};
use runtime_types::RunId;
use sqlx::PgPool;
use std::time::Duration;
use uuid::Uuid;

/// Sends Alice update 42 through the loss proxy concurrently with Bob update
/// 43, then retries 42 directly against Caddy. The caller keeps its existing
/// explicit replay, which raises Alice's duplicate count from one to two.
pub async fn exercise(
    ctx: &IngressLossContext<'_>,
    proxy: IngressLossProxy,
) -> Result<(CookingRun, CookingRun), Box<dyn std::error::Error + Send + Sync>> {
    let baseline_events: i64 =
        sqlx::query_scalar("SELECT count(*) FROM mailbox_events WHERE mailbox_id = $1")
            .bind(ctx.mailbox_id)
            .fetch_one(ctx.pool)
            .await?;
    let mut proxy = proxy;
    let committed = proxy.arm_commit_signal();
    let proxy_url = proxy.url().to_owned();
    let proxy_timeout = ctx.timeout;
    let proxy_task = tokio::spawn(async move {
        tokio::time::timeout(proxy_timeout, proxy.forward_and_drop(proxy_timeout)).await?
    });
    let client = super::super::cooking::caddy_gateway_client_with_timeout(ctx.timeout);
    let request = client
        .post(&proxy_url)
        .header("host", caddy_host(ctx.caddy_public_url)?)
        .header("x-telegram-bot-api-secret-token", ctx.inbound_credential)
        .json(&serde_json::json!({
            "update_id": 42,
            "message": {"from": {"id": 1001}, "text": "pasta"}
        }))
        .send();
    let bob_request = client
        .post(format!(
            "{}/gateway/cooking/telegram",
            ctx.caddy_public_url.trim_end_matches('/')
        ))
        .header("x-telegram-bot-api-secret-token", ctx.inbound_credential)
        .json(&serde_json::json!({
            "update_id": 43,
            "message": {"from": {"id": 1002}, "text": "soup"}
        }))
        .send();
    let (request_result, bob_result, committed_result) =
        tokio::join!(request, bob_request, committed);
    assert!(
        request_result.is_err(),
        "response-loss client unexpectedly received a response"
    );
    let bob = bob_result?;
    assert_eq!(bob.status(), reqwest::StatusCode::OK);
    bob.bytes().await?;
    committed_result.expect("proxy observed the complete Caddy response");
    proxy_task.await??;

    let event_id = wait_for_single_accepted(ctx.pool, ctx.mailbox_id, 42, ctx.timeout).await?;
    wait_for_single_accepted(ctx.pool, ctx.mailbox_id, 43, ctx.timeout).await?;
    assert_eq!(
        mailbox_event_count(ctx.pool, ctx.mailbox_id).await?,
        baseline_events + 2
    );
    let runs_before = wait_for_delivery_attempt(ctx.pool, event_id, ctx.timeout).await?;

    let retry = client
        .post(format!(
            "{}/gateway/cooking/telegram",
            ctx.caddy_public_url.trim_end_matches('/')
        ))
        .header("x-telegram-bot-api-secret-token", ctx.inbound_credential)
        .json(&serde_json::json!({
            "update_id": 42,
            "message": {"from": {"id": 1001}, "text": "pasta"}
        }))
        .send()
        .await?;
    assert_eq!(retry.status(), reqwest::StatusCode::OK);
    retry.bytes().await?;
    wait_for_duplicate(ctx.pool, ctx.mailbox_id, 42, 1, ctx.timeout).await?;
    assert_eq!(event_run_count(ctx.pool, event_id).await?, runs_before);
    assert_eq!(
        mailbox_event_count(ctx.pool, ctx.mailbox_id).await?,
        baseline_events + 2
    );
    let alice_run = wait_for_event_run(ctx.pool, ctx.mailbox_id, 42, ctx.timeout).await?;
    let bob_run = wait_for_event_run(ctx.pool, ctx.mailbox_id, 43, ctx.timeout).await?;
    Ok((alice_run, bob_run))
}

pub(crate) async fn wait_for_single_accepted(
    pool: &PgPool,
    mailbox_id: Uuid,
    update_id: u64,
    timeout: Duration,
) -> Result<Uuid, Box<dyn std::error::Error + Send + Sync>> {
    let key = format!("telegram-update-{update_id}");
    Ok(tokio::time::timeout(timeout, async {
        loop {
            let rows: Vec<(Uuid,)> = sqlx::query_as(
                "SELECT event_id FROM gateway_mailbox_publications
                  WHERE mailbox_id = $1 AND deduplication_key = $2
                    AND outcome = 'accepted'",
            )
            .bind(mailbox_id)
            .bind(&key)
            .fetch_all(pool)
            .await?;
            if rows.len() == 1 {
                return Ok::<Uuid, sqlx::Error>(rows[0].0);
            }
            assert!(
                rows.is_empty(),
                "response-loss ingress created multiple accepted publications"
            );
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await??)
}

pub(crate) async fn wait_for_duplicate(
    pool: &PgPool,
    mailbox_id: Uuid,
    update_id: u64,
    expected: i64,
    timeout: Duration,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let key = format!("telegram-update-{update_id}");
    tokio::time::timeout(timeout, async {
        loop {
            let count: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM gateway_mailbox_publications
                  WHERE mailbox_id = $1 AND deduplication_key = $2
                    AND outcome = 'duplicate'",
            )
            .bind(mailbox_id)
            .bind(&key)
            .fetch_one(pool)
            .await?;
            if count == expected {
                return Ok::<(), sqlx::Error>(());
            }
            assert!(count <= expected);
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await??;
    Ok(())
}

pub(crate) async fn mailbox_event_count(
    pool: &PgPool,
    mailbox_id: Uuid,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT count(*) FROM mailbox_events WHERE mailbox_id = $1")
        .bind(mailbox_id)
        .fetch_one(pool)
        .await
}

pub(crate) async fn event_run_count(pool: &PgPool, event_id: Uuid) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT count(*) FROM mailbox_delivery_attempts WHERE event_id = $1")
        .bind(event_id)
        .fetch_one(pool)
        .await
}

pub(crate) async fn wait_for_delivery_attempt(
    pool: &PgPool,
    event_id: Uuid,
    timeout: Duration,
) -> Result<i64, Box<dyn std::error::Error + Send + Sync>> {
    Ok(tokio::time::timeout(timeout, async {
        loop {
            let count = event_run_count(pool, event_id).await?;
            if count != 0 {
                return Ok::<i64, sqlx::Error>(count);
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await??)
}

pub(crate) async fn wait_for_event_run(
    pool: &PgPool,
    mailbox_id: Uuid,
    update_id: u64,
    timeout: Duration,
) -> Result<CookingRun, Box<dyn std::error::Error + Send + Sync>> {
    let key = format!("telegram-update-{update_id}");
    Ok(tokio::time::timeout(timeout, async {
        loop {
            let row: Option<(Uuid, Uuid)> = sqlx::query_as(
                "SELECT publication.event_id, attempt.run_id
                   FROM gateway_mailbox_publications AS publication
                   JOIN mailbox_delivery_attempts AS attempt
                     ON attempt.event_id = publication.event_id
                   JOIN mailbox_deliveries AS delivery
                     ON delivery.event_id = publication.event_id
                   JOIN runs AS run
                     ON run.id = attempt.run_id
                  WHERE publication.mailbox_id = $1
                    AND publication.deduplication_key = $2
                    AND publication.outcome = 'accepted'
                    AND run.state = 'cleaned_up'
                    AND run.outcome = 'succeeded'
                    AND delivery.disposition = 'delivered'
                    AND NOT EXISTS (
                        SELECT 1
                          FROM mailbox_delivery_attempts AS newer
                         WHERE newer.event_id = attempt.event_id
                           AND (newer.attempt_number, newer.id)
                               > (attempt.attempt_number, attempt.id)
                    )
                  ORDER BY attempt.attempt_number DESC, attempt.id DESC
                  LIMIT 1",
            )
            .bind(mailbox_id)
            .bind(&key)
            .fetch_optional(pool)
            .await?;
            if let Some((event_id, run_id)) = row {
                return Ok::<CookingRun, sqlx::Error>(CookingRun {
                    event_id,
                    run_id: RunId::from_uuid(run_id),
                });
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await??)
}
