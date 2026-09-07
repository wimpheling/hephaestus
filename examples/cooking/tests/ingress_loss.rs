//! Post-commit ingress response-loss probe for the cooking acceptance path.
//!
//! The proxy forwards one exact bounded request to real Caddy, waits for the
//! complete response, then closes the client socket before sending response
//! bytes. SQL is read-only evidence for durable publication and delivery.

use runtime_types::RunId;
use sqlx::PgPool;
use std::{io, net::SocketAddr, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::oneshot,
};
use uuid::Uuid;

use super::cooking::CookingRun;

const MAX_HTTP_MESSAGE_BYTES: usize = 64 * 1024;

pub struct IngressLossContext<'a> {
    pub pool: &'a PgPool,
    pub mailbox_id: Uuid,
    pub caddy_public_url: &'a str,
    pub inbound_credential: &'a str,
    pub timeout: Duration,
}

/// One-shot proxy state. The listener forwards only the accepted request and
/// never writes the upstream response to its client.
pub struct IngressLossProxy {
    listener: TcpListener,
    upstream: SocketAddr,
    url: String,
    committed: Option<oneshot::Sender<()>>,
}

impl IngressLossProxy {
    pub async fn bind(
        caddy_public_url: &str,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let upstream_url = reqwest::Url::parse(caddy_public_url)?;
        let host = upstream_url.host_str().ok_or("Caddy URL has no host")?;
        let port = upstream_url
            .port_or_known_default()
            .ok_or("Caddy URL has no port")?;
        let upstream = tokio::net::lookup_host((host, port))
            .await?
            .next()
            .ok_or("Caddy host did not resolve")?;
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let url = format!("http://{}/gateway/cooking/telegram", listener.local_addr()?);
        Ok(Self {
            listener,
            upstream,
            url,
            committed: None,
        })
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn arm_commit_signal(&mut self) -> oneshot::Receiver<()> {
        let (sender, receiver) = oneshot::channel();
        self.committed = Some(sender);
        receiver
    }

    pub async fn forward_and_drop(
        mut self,
        timeout: Duration,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let operation = async {
            let (mut client, _) = self.listener.accept().await?;
            let request = read_http_message(&mut client, timeout).await?;
            let mut upstream = TcpStream::connect(self.upstream).await?;
            upstream.write_all(&request).await?;
            let response = read_http_message(&mut upstream, timeout).await?;
            let status = response
                .split(|byte| *byte == b' ')
                .nth(1)
                .and_then(|value| std::str::from_utf8(value).ok())
                .unwrap_or_default();
            assert_eq!(status, "200", "Caddy did not accept the forwarded ingress");
            if let Some(signal) = self.committed.take() {
                let _ = signal.send(());
            }
            // Deliberately close before writing even one response byte. The
            // caller therefore observes a transport error after the upstream
            // commit.
            client.shutdown().await?;
            Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
        };
        tokio::time::timeout(timeout, operation).await??;
        Ok(())
    }
}

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
    let client = reqwest::Client::builder().timeout(ctx.timeout).build()?;
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

async fn wait_for_single_accepted(
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

async fn wait_for_duplicate(
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

async fn mailbox_event_count(pool: &PgPool, mailbox_id: Uuid) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT count(*) FROM mailbox_events WHERE mailbox_id = $1")
        .bind(mailbox_id)
        .fetch_one(pool)
        .await
}

async fn event_run_count(pool: &PgPool, event_id: Uuid) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT count(*) FROM mailbox_delivery_attempts WHERE event_id = $1")
        .bind(event_id)
        .fetch_one(pool)
        .await
}

async fn wait_for_delivery_attempt(
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

async fn wait_for_event_run(
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

fn caddy_host(url: &str) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let parsed = reqwest::Url::parse(url)?;
    let host = parsed.host_str().ok_or("Caddy URL has no host")?;
    Ok(parsed
        .port()
        .map_or_else(|| host.to_owned(), |port| format!("{host}:{port}")))
}

async fn read_http_message(
    stream: &mut TcpStream,
    timeout: Duration,
) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    let mut data = Vec::new();
    let mut chunk = [0_u8; 4096];
    let header_end;
    loop {
        let read = tokio::time::timeout(timeout, stream.read(&mut chunk)).await??;
        if read == 0 {
            return Err(
                io::Error::new(io::ErrorKind::UnexpectedEof, "HTTP headers truncated").into(),
            );
        }
        data.extend_from_slice(&chunk[..read]);
        if data.len() > MAX_HTTP_MESSAGE_BYTES {
            return Err(
                io::Error::new(io::ErrorKind::InvalidData, "HTTP message exceeds bound").into(),
            );
        }
        if let Some(position) = data.windows(4).position(|window| window == b"\r\n\r\n") {
            header_end = position + 4;
            break;
        }
    }
    let headers = &data[..header_end];
    let content_length = headers
        .split(|byte| *byte == b'\n')
        .find_map(|line| {
            let line = std::str::from_utf8(line).ok()?.trim();
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        })
        .ok_or("HTTP message lacks bounded Content-Length")?;
    let message_end = header_end
        .checked_add(content_length)
        .filter(|end| *end <= MAX_HTTP_MESSAGE_BYTES)
        .ok_or("HTTP message exceeds bound")?;
    while data.len() < message_end {
        let remaining = message_end - data.len();
        let read_len = remaining.min(chunk.len());
        let read = tokio::time::timeout(timeout, stream.read(&mut chunk[..read_len])).await??;
        if read == 0 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "HTTP body truncated").into());
        }
        data.extend_from_slice(&chunk[..read]);
    }
    data.truncate(message_end);
    Ok(data)
}
