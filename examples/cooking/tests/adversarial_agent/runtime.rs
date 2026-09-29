// Reuse the adversarial facade imports across the focused phases.
#[allow(unused_imports)]
use super::*;
pub(crate) async fn wait_for_mailbox_run(
    pool: &PgPool,
    mailbox_id: Uuid,
    deduplication_key: &str,
    expect_success: bool,
    timeout: Duration,
) -> Result<(Uuid, Uuid), cooking_builds::BuildError> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let row: Option<(Uuid, Uuid, String, Option<String>)> = sqlx::query_as(
            "SELECT event.id, run.id, run.state, run.outcome\n               FROM mailbox_events event\n               JOIN mailbox_delivery_attempts attempt ON attempt.event_id = event.id\n               JOIN runs run ON run.id = attempt.run_id\n              WHERE event.mailbox_id = $1\n                AND event.deduplication_key = $2\n              ORDER BY attempt.attempt_number DESC\n              LIMIT 1",
        )
        .bind(mailbox_id)
        .bind(deduplication_key)
        .fetch_optional(pool)
        .await?;
        if let Some((event_id, run_id, state, outcome)) = row {
            if state == "cleaned_up"
                && outcome.as_deref()
                    == Some(if expect_success {
                        "succeeded"
                    } else {
                        "failed"
                    })
            {
                return Ok((event_id, run_id));
            }
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(invalid(
                "cooking agent run did not reach its expected outcome",
            ));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

pub(crate) fn instance_client(
    running: &RunningHephaestus,
    token_factory: &(dyn Fn(&str) -> String + Send + Sync),
    audience: &str,
) -> Result<AgentInstanceServiceClient<connectrpc::client::HttpClient>, cooking_builds::BuildError>
{
    let uri = format!("http://{}", running.http_addr()).parse()?;
    let config = ClientConfig::new(uri)
        .with_default_header(
            http::header::AUTHORIZATION,
            http::HeaderValue::from_str(&format!("Bearer {}", token_factory(audience)))?,
        )
        .with_default_timeout(Duration::from_secs(30));
    Ok(AgentInstanceServiceClient::new(
        connectrpc::client::HttpClient::plaintext(),
        config,
    ))
}

pub(crate) fn mutation_context(operation: &str) -> RequestContext {
    RequestContext {
        request_id: opaque(Uuid::new_v4()).into(),
        idempotency_key: format!("cooking-adversarial-{operation}-{}", Uuid::new_v4()),
        ..Default::default()
    }
}

pub(crate) fn opaque(value: Uuid) -> OpaqueId {
    OpaqueId {
        value: value.to_string(),
        ..Default::default()
    }
}

pub(crate) fn unique_update_id() -> u64 {
    let bytes = *Uuid::new_v4().as_bytes();
    bounded_update_id(u64::from_be_bytes(
        bytes[..8].try_into().expect("UUID prefix"),
    ))
}

pub(crate) fn bounded_update_id(candidate: u64) -> u64 {
    (candidate & MAX_SIGNED_UPDATE_ID).max(1)
}

pub(crate) const MAX_SIGNED_UPDATE_ID: u64 = 9_223_372_036_854_775_807;

pub(crate) fn invalid(message: &str) -> cooking_builds::BuildError {
    std::io::Error::other(message.to_owned()).into()
}
