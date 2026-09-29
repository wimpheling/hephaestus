use super::*;

/// Waits for the daemon-owned service supervisor to prove readiness and make
/// the exact desired revision active. The query intentionally observes both
/// pointers and the fenced instance state, so a declaration-only Caddy route
/// cannot make the public request proof pass.
pub async fn wait_for_gateway_service_ready(
    pool: &sqlx::PgPool,
    fixture: &GatewayServiceGoldenFixture,
) -> uuid::Uuid {
    tokio::time::timeout(Duration::from_secs(120), async {
        loop {
            let row: Option<(uuid::Uuid, String, Option<uuid::Uuid>)> = sqlx::query_as(
                "SELECT instance.id, instance.state, gateway.active_revision_id
                   FROM gateway_service_instances AS instance
                   JOIN gateways AS gateway ON gateway.id = instance.gateway_id
                  WHERE instance.gateway_id = $1
                    AND instance.revision_id = $2
                  ORDER BY instance.created_at DESC
                  LIMIT 1",
            )
            .bind(fixture.gateway_id)
            .bind(fixture.revision_id)
            .fetch_optional(pool)
            .await
            .expect("read daemon-owned service readiness");
            if let Some((instance_id, state, active_revision_id)) = row {
                if state == "ready" && active_revision_id == Some(fixture.revision_id) {
                    return instance_id;
                }
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("daemon-owned service reaches Ready and active state")
}

/// Waits for the daemon to retain the crashed instance's redacted exit report,
/// finish its physical cleanup, and promote a replacement for the same
/// immutable revision.
pub async fn wait_for_gateway_service_crash_replacement(
    pool: &sqlx::PgPool,
    fixture: &GatewayServiceGoldenFixture,
    crashed_instance_id: uuid::Uuid,
) -> uuid::Uuid {
    tokio::time::timeout(Duration::from_secs(120), async {
        loop {
            let old: Option<GatewayServiceCrashEvidence> = sqlx::query_as(
                "SELECT state, failure_code, exit_code, exit_signal
                       FROM gateway_service_instances
                      WHERE id = $1 AND gateway_id = $2 AND revision_id = $3",
            )
            .bind(crashed_instance_id)
            .bind(fixture.gateway_id)
            .bind(fixture.revision_id)
            .fetch_optional(pool)
            .await
            .expect("read crashed persistent-service instance");
            let replacement: Option<uuid::Uuid> = sqlx::query_scalar(
                "SELECT instance.id
                   FROM gateway_service_instances AS instance
                   JOIN gateways AS gateway ON gateway.id = instance.gateway_id
                  WHERE instance.id <> $1
                    AND instance.gateway_id = $2
                    AND instance.revision_id = $3
                    AND instance.state = 'ready'
                    AND gateway.active_revision_id = $3
                  ORDER BY instance.created_at DESC
                  LIMIT 1",
            )
            .bind(crashed_instance_id)
            .bind(fixture.gateway_id)
            .bind(fixture.revision_id)
            .fetch_optional(pool)
            .await
            .expect("read replacement persistent-service instance");
            if let (
                Some((state, Some(failure_code), Some(exit_code), None)),
                Some(replacement_id),
            ) = (old, replacement)
            {
                if state == "cleaned" && failure_code == "unexpected_exit" && exit_code == 42 {
                    return replacement_id;
                }
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("daemon replaces crashed persistent service")
}

/// Sends the opt-in fixture crash request through public Caddy routing and
/// waits for the guest's acknowledged 503 before it exits with code 42.
pub async fn exercise_gateway_service_crash(public_url: &str) {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("bounded persistent-service crash client");
    let response = client
        .get(format!("{public_url}/gateway/service/crash"))
        .send()
        .await
        .expect("public persistent-service crash request");
    assert_eq!(response.status(), reqwest::StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        response
            .bytes()
            .await
            .expect("read persistent-service crash response"),
        "crashing"
    );
}
