use super::*;

#[allow(clippy::needless_borrow, clippy::too_many_lines)]
pub async fn run_adversarial_gateway_denial(prep: &mut CookingPreparation) {
    let CookingPreparation {
        pool,
        running: running_slot,
        app_config: app_config_slot,
        adversarial_configured,
        adversarial_foreign_mailbox,
        ..
    } = prep;
    let running = running_slot.take().expect("cooking daemon before probe");
    let app_config = app_config_slot.take().expect("cooking config before probe");
    let pool = &*pool;
    let adversarial_configured = &*adversarial_configured;
    let adversarial_foreign_mailbox = *adversarial_foreign_mailbox;
    running
        .shutdown()
        .await
        .expect("build-proof daemon restart shutdown");
    let running = Box::pin(restart_application(app_config.clone())).await;
    let adversarial_url = format!(
        "{}/gateway/cooking/telegram",
        env::var("HEPHAESTUS_CADDY_TEST_PUBLIC_URL").expect("public Caddy URL")
    );
    let adversarial_response = cooking::caddy_gateway_client()
        .post(adversarial_url)
        .header("x-telegram-bot-api-secret-token", cooking::INBOUND_SENTINEL)
        .json(&serde_json::json!({
            "update_id": 39,
            "message": {"from": {"id": 1001}, "text": "pasta"}
        }))
        .send()
        .await
        .expect("adversarial foreign publication request");
    assert_eq!(
        adversarial_response.status(),
        reqwest::StatusCode::BAD_GATEWAY,
        "undeclared gateway publication slot is denied by the host"
    );
    adversarial_response
        .bytes()
        .await
        .expect("read adversarial denial response");
    let denied_publications: Vec<(String, Option<String>, String)> = sqlx::query_as(
        "SELECT publication.outcome, publication.denial_code, invocation.outcome
           FROM gateway_mailbox_publications publication
           JOIN gateway_invocations invocation ON invocation.id = publication.invocation_id
          WHERE publication.gateway_revision_id = $1
            AND publication.slot_key = $2
            AND publication.deduplication_key = 'telegram-update-39'",
    )
    .bind(adversarial_configured.revision_id)
    .bind(cooking_builds::FOREIGN_PUBLICATION_SLOT)
    .fetch_all(pool)
    .await
    .expect("exact adversarial publication denial");
    assert_eq!(
        denied_publications,
        vec![(
            String::from("denied"),
            Some(String::from("authority_unavailable")),
            String::from("failed"),
        )],
        "the guest must reach publication and receive a durable authority denial"
    );
    let (foreign_events, foreign_deliveries, foreign_attempts, foreign_runs): (i64, i64, i64, i64) =
        sqlx::query_as(
            "SELECT
            (SELECT count(*) FROM mailbox_events WHERE mailbox_id = $1),
            (SELECT count(*) FROM mailbox_deliveries WHERE mailbox_id = $1),
            (SELECT count(*) FROM mailbox_delivery_attempts WHERE mailbox_id = $1),
            (SELECT count(*) FROM runs WHERE instance_id = (
                SELECT instance_id FROM mailboxes WHERE id = $1
            ))",
        )
        .bind(adversarial_foreign_mailbox)
        .fetch_one(pool)
        .await
        .expect("foreign mailbox denial effects");
    assert_eq!(
        (
            foreign_events,
            foreign_deliveries,
            foreign_attempts,
            foreign_runs
        ),
        (0, 0, 0, 0),
        "foreign mailbox must have no event, delivery, attempt, or run after denial"
    );

    *running_slot = Some(running);
    *app_config_slot = Some(app_config);
}
