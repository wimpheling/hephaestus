use super::*;

/// Runs the joined Caddy request, delivery, denial, and shutdown proof.
#[allow(
    clippy::cognitive_complexity,
    clippy::needless_return,
    clippy::ref_option,
    clippy::too_many_arguments,
    clippy::too_many_lines
)]
pub async fn run_libkrun_gateway_requests(
    pool: &sqlx::PgPool,
    nats_url: &str,
    gateway_caddy_e2e: bool,
    gateway_edge: &Option<(GatewayEdgeConfig, GatewayGoldenFixture)>,
    gateway_service_fixture: &Option<GatewayServiceGoldenFixture>,
    public_url: Option<String>,
    gateway_service_e2e: bool,
    user_id: UserId,
    running: hephaestus_app::RunningHephaestus,
    brokered_fixture: &mut Option<BrokeredFixture>,
    service_instance_id: Option<uuid::Uuid>,
    service_resource_paths: Option<(PathBuf, PathBuf, PathBuf)>,
) {
    if gateway_caddy_e2e && gateway_service_e2e {
        let public_url = public_url.as_deref().expect("joined public URL");
        let client = cooking::caddy_gateway_client();
        let first = client
            .post(format!("{public_url}/gateway/brokered?mode=real"))
            .header("x-webhook-secret", BROKERED_E2E_SENTINEL)
            .body("gateway-brokered-request-a")
            .send();
        let second = client
            .post(format!("{public_url}/gateway/brokered?mode=real"))
            .header("x-webhook-secret", BROKERED_E2E_SENTINEL)
            .body("gateway-brokered-request-b")
            .send();
        let (response, distinct) = tokio::join!(first, second);
        let response = response.expect("first concurrent Caddy gateway request");
        let distinct = distinct.expect("second concurrent Caddy gateway request");
        if response.status() != reqwest::StatusCode::CREATED
            || distinct.status() != reqwest::StatusCode::CREATED
        {
            let gateway_diagnostics: (i64, i64, i64, i64, i64, Vec<String>, Vec<String>) =
                sqlx::query_as(
                    "SELECT
                         (SELECT count(*) FROM gateway_invocations),
                         (SELECT count(*) FROM gateway_authorization_snapshots),
                         (SELECT count(*) FROM gateway_authorization_snapshot_bindings),
                         (SELECT count(*) FROM gateway_runtime_authority_sessions),
                         (SELECT count(*) FROM gateway_mailbox_publications),
                         COALESCE((SELECT array_agg(status ORDER BY id)
                                   FROM gateway_runtime_authority_sessions), ARRAY[]::text[]),
                         COALESCE((SELECT array_agg(outcome ORDER BY id)
                                   FROM gateway_invocations), ARRAY[]::text[])",
                )
                .fetch_one(pool)
                .await
                .expect("load gateway failure diagnostics");
            panic!(
                "joined gateway requests failed: first={}, second={}, invocations={}, snapshots={}, snapshot_bindings={}, sessions={}, publications={}, session_statuses={:?}, invocation_outcomes={:?}",
                response.status(),
                distinct.status(),
                gateway_diagnostics.0,
                gateway_diagnostics.1,
                gateway_diagnostics.2,
                gateway_diagnostics.3,
                gateway_diagnostics.4,
                gateway_diagnostics.5,
                gateway_diagnostics.6,
            );
        }
        assert_eq!(response.status(), reqwest::StatusCode::CREATED);
        assert_eq!(distinct.status(), reqwest::StatusCode::CREATED);
        assert_eq!(
            response.bytes().await.expect("gateway response body"),
            "gateway-brokered-header-ok"
        );
        assert_eq!(
            distinct
                .bytes()
                .await
                .expect("distinct gateway response body"),
            "gateway-brokered-header-ok"
        );
        // A handler retry emits the same application-supplied key. It is
        // a fresh gateway invocation but must retain the first logical
        // mailbox event rather than delivering a second one.
        let retry = client
            .post(format!("{public_url}/gateway/brokered?mode=real"))
            .header("x-webhook-secret", BROKERED_E2E_SENTINEL)
            .body("gateway-brokered-request-a")
            .send()
            .await
            .expect("Caddy gateway retry");
        assert_eq!(retry.status(), reqwest::StatusCode::CREATED);
        assert_eq!(
            retry.bytes().await.expect("gateway retry response body"),
            "gateway-brokered-header-ok"
        );
        let evidence: (String, bool, bool) = sqlx::query_as(
            "SELECT invocation.outcome, session.acknowledged_at IS NOT NULL,
                        EXISTS (
                            SELECT 1 FROM gateway_secret_leases AS lease
                             WHERE lease.invocation_id = invocation.id
                        )
                   FROM gateway_invocations AS invocation
                   JOIN gateway_runtime_authority_sessions AS session
                     ON session.invocation_id = invocation.id
                  WHERE invocation.request_id IS NOT NULL
                  ORDER BY invocation.accepted_at DESC LIMIT 1",
        )
        .fetch_one(pool)
        .await
        .expect("persisted joined gateway authority evidence");
        assert_eq!(evidence.0, "completed");
        assert!(evidence.1, "gateway VM acknowledged its runtime authority");
        assert!(
            evidence.2,
            "gateway invocation held its inbound secret lease"
        );
        let mailbox = gateway_edge
            .as_ref()
            .expect("joined gateway fixture")
            .1
            .mailbox_id;
        let (accepted_publications, duplicate_publications): (i64, i64) = sqlx::query_as(
            "SELECT count(*) FILTER (WHERE publication.outcome = 'accepted'),
                        count(*) FILTER (WHERE publication.outcome = 'duplicate')
                   FROM gateway_mailbox_publications AS publication
                   JOIN gateway_invocations AS invocation
                     ON invocation.id = publication.invocation_id
                  WHERE publication.mailbox_id = $1",
        )
        .bind(mailbox.as_uuid())
        .fetch_one(pool)
        .await
        .expect("gateway mailbox publication provenance");
        assert_eq!((accepted_publications, duplicate_publications), (2, 1));
        let (event_count, wake_count, leaked_body): (i64, i64, bool) = sqlx::query_as(
            "SELECT
                     (SELECT count(*) FROM mailbox_events WHERE mailbox_id = $1),
                     (SELECT count(*) FROM outbox
                       WHERE subject = 'heph.mailbox.v1.wake'
                         AND id IN (SELECT id FROM mailbox_events WHERE mailbox_id = $1)),
                     EXISTS (
                       SELECT 1 FROM gateway_mailbox_publications
                        WHERE mailbox_id = $1
                          AND to_jsonb(gateway_mailbox_publications)::text
                              LIKE '%gateway-real-mailbox-body%'
                     )",
        )
        .bind(mailbox.as_uuid())
        .fetch_one(pool)
        .await
        .expect("gateway mailbox acceptance evidence");
        assert_eq!((event_count, wake_count), (2, 2));
        assert!(!leaked_body, "gateway publication provenance is value-free");

        // This waits for the event emitted by the real libkrun guest to
        // cross the transactional outbox and JetStream dispatcher into a
        // separate real target-agent run. The target command rejects an
        // altered route or body before it can succeed.
        let delivery = tokio::time::timeout(Duration::from_secs(60), async {
            loop {
                let row = sqlx::query_as::<_, (i64, i64)>(
                    "SELECT count(DISTINCT delivery.event_id)
                                  FILTER (WHERE delivery.disposition = 'delivered'),
                                count(*) FILTER (
                                    WHERE run.state = 'cleaned_up'
                                      AND run.outcome = 'succeeded'
                                )
                           FROM mailbox_deliveries AS delivery
                           JOIN mailbox_delivery_attempts AS attempt
                             ON attempt.event_id = delivery.event_id
                           JOIN runs AS run ON run.id = attempt.run_id
                          WHERE delivery.mailbox_id = $1",
                )
                .bind(mailbox.as_uuid())
                .fetch_one(pool)
                .await
                .expect("gateway mailbox delivery evidence");
                if row == (2, 2) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await;
        if delivery.is_err() {
            let failures: Vec<(String, String, Option<String>, Option<String>)> = sqlx::query_as(
                "SELECT delivery.disposition, run.state, run.outcome, run.failure
                     FROM mailbox_delivery_attempts AS attempt
                     JOIN mailbox_deliveries AS delivery ON delivery.event_id = attempt.event_id
                     JOIN runs AS run ON run.id = attempt.run_id
                     WHERE delivery.mailbox_id = $1 ORDER BY run.id",
            )
            .bind(mailbox.as_uuid())
            .fetch_all(pool)
            .await
            .expect("gateway delivery failures");
            panic!("real gateway publications reach target-agent dispatch: {failures:?}");
        }

        // New invocations are denied after the exact bound grant is
        // revoked. This runs after the already accepted events settled so
        // it proves live authorization without perturbing their delivery.
        let fixture = &gateway_edge.as_ref().expect("joined gateway fixture").1;
        sqlx::query(
            "UPDATE gateway_mailbox_binding_grants
                    SET status = 'revoked', revoked_at = now(), revoked_by = $2
                  WHERE id = $1",
        )
        .bind(fixture.grant_id)
        .bind(user_id.as_uuid())
        .execute(pool)
        .await
        .expect("revoke bound gateway mailbox grant");
        let denied = client
            .post(format!("{public_url}/gateway/brokered?mode=real"))
            .header("x-webhook-secret", BROKERED_E2E_SENTINEL)
            .body("gateway-brokered-request-denied")
            .send()
            .await
            .expect("revoked Caddy gateway request");
        assert_eq!(denied.status(), reqwest::StatusCode::BAD_GATEWAY);
        let accepted_after_revoke: i64 =
            sqlx::query_scalar("SELECT count(*) FROM mailbox_events WHERE mailbox_id = $1")
                .bind(mailbox.as_uuid())
                .fetch_one(pool)
                .await
                .expect("revoked grant does not add mailbox events");
        assert_eq!(accepted_after_revoke, 2);
    }
    if let Some(fixture) = brokered_fixture.take() {
        fixture.upstream.assert_substituted_request().await;
    }
    running.shutdown().await.expect("graceful daemon shutdown");
    if let (
        Some(service_fixture),
        Some(instance_id),
        Some((provider_runtime, cgroup, materializer)),
    ) = (
        gateway_service_fixture.as_ref(),
        service_instance_id,
        service_resource_paths,
    ) {
        let state: Option<String> = sqlx::query_scalar(
            "SELECT state FROM gateway_service_instances
              WHERE id = $1 AND gateway_id = $2 AND revision_id = $3",
        )
        .bind(instance_id)
        .bind(service_fixture.gateway_id)
        .bind(service_fixture.revision_id)
        .fetch_optional(pool)
        .await
        .expect("read cleaned persistent-service instance");
        assert_eq!(state.as_deref(), Some("cleaned"));
        assert!(!provider_runtime.exists());
        assert!(!cgroup.exists());
        assert!(!materializer.exists());
    }
    if gateway_service_e2e {
        println!("persistent-service-e2e=passed");
    }
    cleanup_streams(nats_url).await;
    return;
}
