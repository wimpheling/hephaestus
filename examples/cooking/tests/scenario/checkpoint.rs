// Reuse the scenario facade imports so each phase preserves the production fixture context.
#[allow(unused_imports)]
use super::*;
#[derive(Clone, Copy)]
pub(crate) struct CookingRun {
    pub event_id: uuid::Uuid,
    pub run_id: runtime_types::RunId,
}

pub(crate) type CookingRunRow = (
    uuid::Uuid,
    uuid::Uuid,
    String,
    Option<String>,
    Option<String>,
);

pub(crate) type CookingRunSuccessRow = (uuid::Uuid, uuid::Uuid, String, Option<String>);

#[derive(Clone, Copy)]
pub(crate) struct CookingRunObservation {
    pub(crate) event_id: uuid::Uuid,
    pub(crate) run_id: uuid::Uuid,
    pub(crate) state: &'static str,
    pub(crate) outcome: &'static str,
}

pub(crate) fn safe_run_state(value: &str) -> &'static str {
    match value {
        "queued" => "queued",
        "leasing_volume" => "leasing_volume",
        "provisioning" => "provisioning",
        "starting" => "starting",
        "running" => "running",
        "succeeded" => "succeeded",
        "failed" => "failed",
        "cancelled" => "cancelled",
        "cleaned_up" => "cleaned_up",
        _ => "unknown",
    }
}

pub(crate) fn safe_run_outcome(value: Option<&str>) -> &'static str {
    match value {
        None => "none",
        Some("succeeded") => "succeeded",
        Some("failed") => "failed",
        Some("cancelled") => "cancelled",
        Some(_) => "unknown",
    }
}

/// Runs admitted before the daemon restart; the follow-up reuses this frozen
/// provenance after a fresh supervisor boot.
pub(crate) struct CookingCheckpoint {
    pub(crate) alice: CookingRun,
    pub(crate) bob: CookingRun,
}

impl CookingCheckpoint {
    /// Returns the exact canonical operations whose broker effects form the
    /// post-ingress positive-control baseline.
    pub const fn run_ids(&self) -> [uuid::Uuid; 2] {
        [self.alice.run_id.as_uuid(), self.bob.run_id.as_uuid()]
    }
}

/// Waits until both canonical positive-control operations have reached their
/// terminal cleanup state before an audit baseline is captured.
pub(crate) async fn wait_for_checkpoint_runs(
    pool: &sqlx::PgPool,
    checkpoint: &CookingCheckpoint,
    timeout: Duration,
) {
    let run_ids = checkpoint.run_ids();
    tokio::time::timeout(timeout, async {
        loop {
            let settled: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM runs
                  WHERE id = ANY($1)
                    AND state = 'cleaned_up'
                    AND outcome = 'succeeded'",
            )
            .bind(run_ids.to_vec())
            .fetch_one(pool)
            .await
            .expect("canonical cooking runs settle");
            if settled == i64::try_from(run_ids.len()).expect("checkpoint size fits i64") {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("canonical cooking runs cleanup");
}

pub(crate) async fn send_update(
    client: &reqwest::Client,
    url: &str,
    update_id: u64,
    provider_id: u64,
    text: &str,
) -> reqwest::Response {
    send_update_with_credential(client, url, update_id, provider_id, text, INBOUND_SENTINEL).await
}

/// Sends one normalized cooking update with an explicitly selected inbound
/// credential. The credential parameter exists only for the rotation probe;
/// ordinary scenario requests continue through [`send_update`].
pub(crate) async fn send_update_with_credential(
    client: &reqwest::Client,
    url: &str,
    update_id: u64,
    provider_id: u64,
    text: &str,
    credential: &str,
) -> reqwest::Response {
    client
        .post(url)
        .header("x-telegram-bot-api-secret-token", credential)
        .json(&serde_json::json!({
            "update_id": update_id,
            "message": {"from": {"id": provider_id}, "text": text}
        }))
        .send()
        .await
        .expect("cooking ingress request")
}

pub(crate) async fn send_raw(
    client: &reqwest::Client,
    url: &str,
    body: Vec<u8>,
    credential: Option<&str>,
) -> reqwest::Response {
    let mut request = client.post(url).body(body);
    if let Some(value) = credential {
        request = request.header("x-telegram-bot-api-secret-token", value);
    }
    request.send().await.expect("cooking ingress request")
}

pub(crate) async fn mailbox_counts(
    pool: &sqlx::PgPool,
    gateway: &GatewayGoldenFixture,
) -> (i64, i64, i64) {
    sqlx::query_as(
        "SELECT count(*) FILTER (WHERE outcome = 'accepted'),
                count(*) FILTER (WHERE outcome = 'duplicate'),
                (SELECT count(*) FROM mailbox_events WHERE mailbox_id = $1)
           FROM gateway_mailbox_publications
          WHERE mailbox_id = $1",
    )
    .bind(gateway.mailbox_id.as_uuid())
    .fetch_one(pool)
    .await
    .expect("cooking mailbox publication counts")
}

pub(crate) async fn assert_normalized_mailbox_body(
    pool: &sqlx::PgPool,
    event_id: uuid::Uuid,
    update_id: u64,
    user_id: &str,
    text: &str,
) {
    let (method, route, headers, content_type, body): (
        String,
        String,
        serde_json::Value,
        Option<String>,
        Vec<u8>,
    ) = sqlx::query_as(
        "SELECT event.method, event.route, event.selected_headers,
                event.content_type, payload.encoded_body
           FROM mailbox_events event
           JOIN mailbox_payloads payload ON payload.id = event.body_id
          WHERE event.id = $1",
    )
    .bind(event_id)
    .fetch_one(pool)
    .await
    .expect("normalized mailbox envelope");
    assert_eq!(method, "POST");
    assert_eq!(route, "/telegram/updates");
    assert_eq!(
        headers,
        serde_json::json!({
            "x-cooking-event-kind": "cooking.telegram.received.v1"
        })
    );
    assert_eq!(content_type.as_deref(), Some("application/json"));
    let normalized: serde_json::Value =
        serde_json::from_slice(&body).expect("normalized cooking event JSON");
    assert_eq!(
        normalized,
        serde_json::json!({
            "provider_update_id": update_id,
            "user_id": user_id,
            "command": "recipe",
            "text": text,
        })
    );
    assert!(
        !normalized
            .to_string()
            .contains("telegram-bot-api-secret-token")
    );
    assert_no_credentials(&normalized.to_string());
}
