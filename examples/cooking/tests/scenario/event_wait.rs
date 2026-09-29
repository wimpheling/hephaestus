// Reuse the scenario facade imports so each phase preserves the production fixture context.
#[allow(unused_imports)]
use super::*;
pub(crate) async fn wait_for_event_run(
    pool: &sqlx::PgPool,
    gateway: &GatewayGoldenFixture,
    update_id: u64,
) -> CookingRun {
    let key = format!("telegram-update-{update_id}");
    let mut latest_observation = None;
    let mut next_lineage_snapshot = tokio::time::Instant::now();
    let result = tokio::time::timeout(Duration::from_secs(90), async {
        loop {
            if tokio::time::Instant::now() >= next_lineage_snapshot {
                write_cooking_lineage_snapshot(pool, gateway.mailbox_id.as_uuid(), None).await;
                next_lineage_snapshot = tokio::time::Instant::now() + Duration::from_secs(1);
            }
            let row: Option<CookingRunSuccessRow> = sqlx::query_as(
                "SELECT publication.event_id, run.id, run.state, run.outcome
                   FROM gateway_mailbox_publications publication
                   JOIN mailbox_events event ON event.id = publication.event_id
                   JOIN mailbox_delivery_attempts attempt ON attempt.event_id = event.id
                   JOIN runs run ON run.id = attempt.run_id
                  WHERE publication.mailbox_id = $1
                    AND publication.deduplication_key = $2
                    AND publication.outcome IN ('accepted', 'duplicate')
                  ORDER BY attempt.attempt_number DESC
                  LIMIT 1",
            )
            .bind(gateway.mailbox_id.as_uuid())
            .bind(&key)
            .fetch_optional(pool)
            .await
            .expect("cooking run by exact event key");
            if let Some((event_id, run_id, state, outcome)) = row {
                latest_observation = Some(CookingRunObservation {
                    event_id,
                    run_id,
                    state: safe_run_state(&state),
                    outcome: safe_run_outcome(outcome.as_deref()),
                });
                if state == "cleaned_up" && outcome.as_deref() == Some("succeeded") {
                    break CookingRun {
                        event_id,
                        run_id: runtime_types::RunId::from_uuid(run_id),
                    };
                }
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await;
    result.unwrap_or_else(|_| {
        let latest = latest_observation.map_or_else(
            || String::from("none"),
            |observation| {
                format!(
                    "event_id={} run_id={} state={} outcome={}",
                    observation.event_id,
                    observation.run_id,
                    observation.state,
                    observation.outcome
                )
            },
        );
        panic!(
            "cooking mailbox run completes: mailbox_id={} update_id={} latest={latest}",
            gateway.mailbox_id.as_uuid(),
            update_id
        );
    })
}

/// Waits for one exact mailbox event to fail after its run has cleaned up.
/// The returned failure is retained so the caller can assert the typed
/// broker-denial category without exposing provider values in diagnostics.
pub(crate) async fn wait_for_failed_event_run(
    pool: &sqlx::PgPool,
    gateway: &GatewayGoldenFixture,
    update_id: u64,
) -> CookingRun {
    let key = format!("telegram-update-{update_id}");
    let mut next_lineage_snapshot = tokio::time::Instant::now();
    tokio::time::timeout(Duration::from_secs(90), async {
        loop {
            if tokio::time::Instant::now() >= next_lineage_snapshot {
                write_cooking_lineage_snapshot(pool, gateway.mailbox_id.as_uuid(), None).await;
                next_lineage_snapshot = tokio::time::Instant::now() + Duration::from_secs(1);
            }
            let row: Option<CookingRunRow> = sqlx::query_as(
                "SELECT publication.event_id, run.id, run.state, run.outcome, run.failure
                   FROM gateway_mailbox_publications publication
                   JOIN mailbox_events event ON event.id = publication.event_id
                   JOIN mailbox_delivery_attempts attempt ON attempt.event_id = event.id
                   JOIN runs run ON run.id = attempt.run_id
                  WHERE publication.mailbox_id = $1
                    AND publication.deduplication_key = $2
                    AND publication.outcome = 'accepted'
                  ORDER BY attempt.attempt_number DESC LIMIT 1",
            )
            .bind(gateway.mailbox_id.as_uuid())
            .bind(&key)
            .fetch_optional(pool)
            .await
            .expect("failed cooking run by exact event key");
            if let Some((event_id, run_id, state, outcome, _failure)) = row {
                if state == "cleaned_up" && outcome.as_deref() == Some("failed") {
                    break CookingRun {
                        event_id,
                        run_id: runtime_types::RunId::from_uuid(run_id),
                    };
                }
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("revoked relay cooking run completes")
}

/// Holds a real model operation, revokes the relay source while its run and
/// lease are active, then releases the model. The run must fail at broker
/// admission before any relay request or relay ledger mutation occurs.
pub(crate) async fn exercise_active_relay_revocation(
    pool: &sqlx::PgPool,
    gateway: &GatewayGoldenFixture,
    actor: UserId,
    upstream: &BrokeredTlsUpstream,
    relay_rule_id: uuid::Uuid,
) {
    let public = env::var("HEPHAESTUS_CADDY_TEST_PUBLIC_URL").expect("public Caddy URL");
    let url = format!("{public}/gateway/cooking/telegram");
    let client = caddy_gateway_client();
    let accepted = send_update_with_credential(
        &client,
        &url,
        50,
        1001,
        "revoked-relay",
        INBOUND_ROTATED_SENTINEL,
    )
    .await;
    assert_eq!(accepted.status(), reqwest::StatusCode::OK);
    accepted
        .bytes()
        .await
        .expect("active revocation acknowledgement");
    upstream.wait_revocation_entered().await;
    let active = wait_for_active_event_run(
        pool,
        gateway.mailbox_id.as_uuid(),
        Duration::from_secs(90),
        50,
    )
    .await;
    let active_relay_leases: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM brokered_secret_lease_snapshots snapshot
          JOIN secret_leases lease ON lease.id = snapshot.lease_id
         WHERE snapshot.run_id = $1 AND snapshot.rule_id = $2
           AND lease.status = 'active' AND lease.expires_at > now()",
    )
    .bind(active.run_id.as_uuid())
    .bind(relay_rule_id)
    .fetch_one(pool)
    .await
    .expect("active relay lease before revocation");
    assert_eq!(
        active_relay_leases, 1,
        "relay lease is pinned before revoke"
    );
    let relay_import = import_id_for_brokered_rule(pool, relay_rule_id).await;
    revoke_imported_credential(pool, actor, relay_import).await;
    upstream.release_revocation();
    let failed = wait_for_failed_event_run(pool, gateway, 50).await;
    assert_eq!(failed.event_id, active.event_id);
    assert_eq!(
        failed.run_id, active.run_id,
        "relay denial must settle the held operation rather than create a replacement run"
    );
    let denied_decisions: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM brokered_secret_audit_events
          WHERE run_id = $1 AND rule_id = $2
            AND event_kind = 'authorization_decision' AND decision = 'deny'",
    )
    .bind(failed.run_id.as_uuid())
    .bind(relay_rule_id)
    .fetch_one(pool)
    .await
    .expect("typed relay broker denial audit");
    assert_eq!(denied_decisions, 1, "relay denial is durably typed");
    let relay_uses: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM brokered_secret_audit_events
          WHERE run_id = $1 AND rule_id = $2 AND event_kind = 'substitution_use'",
    )
    .bind(failed.run_id.as_uuid())
    .bind(relay_rule_id)
    .fetch_one(pool)
    .await
    .expect("revoked relay substitution audit");
    assert_eq!(
        relay_uses, 0,
        "denied relay has no logical substitution use"
    );
    let relay_attempts: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM brokered_secret_lease_snapshots
          WHERE run_id = $1 AND rule_id = $2",
    )
    .bind(failed.run_id.as_uuid())
    .bind(relay_rule_id)
    .fetch_one(pool)
    .await
    .expect("revoked relay lease history");
    assert_eq!(
        relay_attempts, 1,
        "the denied run retains one pinned relay lease"
    );
}

pub(crate) async fn wait_for_event_id(
    pool: &sqlx::PgPool,
    mailbox_id: uuid::Uuid,
    timeout: Duration,
    update_id: u64,
) -> uuid::Uuid {
    let key = format!("telegram-update-{update_id}");
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let event_id: Option<uuid::Uuid> = sqlx::query_scalar(
            "SELECT event_id FROM gateway_mailbox_publications
              WHERE mailbox_id = $1
                AND deduplication_key = $2 AND outcome = 'accepted'
              ORDER BY accepted_at DESC LIMIT 1",
        )
        .bind(mailbox_id)
        .bind(&key)
        .fetch_optional(pool)
        .await
        .expect("deferred cooking event ID");
        if let Some(event_id) = event_id {
            return event_id;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "cooking event publication timed out"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

pub(crate) async fn wait_for_active_event_run(
    pool: &sqlx::PgPool,
    mailbox_id: uuid::Uuid,
    timeout: Duration,
    update_id: u64,
) -> CookingRun {
    let key = format!("telegram-update-{update_id}");
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let row: Option<(uuid::Uuid, uuid::Uuid, String)> = sqlx::query_as(
            "SELECT publication.event_id, run.id, run.state
               FROM gateway_mailbox_publications publication
               JOIN mailbox_delivery_attempts attempt ON attempt.event_id = publication.event_id
               JOIN runs run ON run.id = attempt.run_id
              WHERE publication.mailbox_id = $1
                AND publication.deduplication_key = $2
                AND publication.outcome = 'accepted'
              ORDER BY attempt.attempt_number DESC LIMIT 1",
        )
        .bind(mailbox_id)
        .bind(&key)
        .fetch_optional(pool)
        .await
        .expect("active cooking event run");
        if let Some((event_id, run_id, state)) = row {
            if [
                "queued",
                "leasing_volume",
                "provisioning",
                "starting",
                "running",
            ]
            .contains(&state.as_str())
            {
                return CookingRun {
                    event_id,
                    run_id: runtime_types::RunId::from_uuid(run_id),
                };
            }
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "active cooking event run timed out"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}
