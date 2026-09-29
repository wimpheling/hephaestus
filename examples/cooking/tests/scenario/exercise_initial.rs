// Reuse the scenario facade imports so each phase preserves the production fixture context.
#[allow(unused_imports)]
use super::*;
#[allow(clippy::cognitive_complexity)]
pub(crate) async fn exercise_initial(
    pool: &sqlx::PgPool,
    gateway: &GatewayGoldenFixture,
) -> CookingCheckpoint {
    let public = env::var("HEPHAESTUS_CADDY_TEST_PUBLIC_URL").expect("public Caddy URL");
    let url = format!("{public}/gateway/cooking/telegram");
    let client = caddy_gateway_client();
    let baseline = mailbox_counts(pool, gateway).await;
    for credential in [None, Some("wrong-inbound-value")] {
        let response = send_raw(
            &client,
            &url,
            serde_json::to_vec(&serde_json::json!({
                "update_id": 40,
                "message": {"from": {"id": 1001}, "text": "pasta"}
            }))
            .expect("invalid verification body"),
            credential,
        )
        .await;
        assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
        assert!(response.bytes().await.expect("empty denial").is_empty());
        assert_eq!(mailbox_counts(pool, gateway).await, baseline);
    }
    let unknown = send_raw(
        &client,
        &url,
        serde_json::to_vec(&serde_json::json!({
            "update_id": 41,
            "message": {"from": {"id": 9999}, "text": "pasta"}
        }))
        .expect("unknown identity body"),
        Some(INBOUND_SENTINEL),
    )
    .await;
    assert_eq!(unknown.status(), reqwest::StatusCode::FORBIDDEN);
    unknown.bytes().await.expect("unknown identity response");
    assert_eq!(mailbox_counts(pool, gateway).await, baseline);

    let malformed = send_raw(&client, &url, b"{".to_vec(), Some(INBOUND_SENTINEL)).await;
    assert_eq!(malformed.status(), reqwest::StatusCode::BAD_REQUEST);
    malformed.bytes().await.expect("malformed response");
    assert_eq!(mailbox_counts(pool, gateway).await, baseline);
    let oversized = send_raw(&client, &url, vec![b'x'; 16_385], Some(INBOUND_SENTINEL)).await;
    assert_eq!(oversized.status(), reqwest::StatusCode::BAD_REQUEST);
    oversized.bytes().await.expect("oversized response");
    assert_eq!(mailbox_counts(pool, gateway).await, baseline);

    let proxy = super::super::cooking_ingress_loss::IngressLossProxy::bind(&public)
        .await
        .expect("bind post-commit ingress response-loss proxy");
    let (alice, bob) = super::super::cooking_ingress_loss::exercise(
        &super::super::cooking_ingress_loss::IngressLossContext {
            pool,
            mailbox_id: gateway.mailbox_id.as_uuid(),
            caddy_public_url: &public,
            inbound_credential: INBOUND_SENTINEL,
            timeout: Duration::from_secs(90),
        },
        proxy,
    )
    .await
    .expect("post-commit ingress response-loss and replay");
    assert_ne!(alice.event_id, bob.event_id);
    assert_normalized_mailbox_body(pool, alice.event_id, 42, "alice", "pasta").await;
    assert_normalized_mailbox_body(pool, bob.event_id, 43, "bob", "soup").await;

    let duplicate = send_update(&client, &url, 42, 1001, "pasta").await;
    assert_eq!(duplicate.status(), reqwest::StatusCode::OK);
    duplicate.bytes().await.expect("duplicate acknowledgement");
    let duplicate_count = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let count: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM gateway_mailbox_publications
                  WHERE mailbox_id = $1 AND deduplication_key = 'telegram-update-42'
                    AND outcome = 'duplicate'",
            )
            .bind(gateway.mailbox_id.as_uuid())
            .fetch_one(pool)
            .await
            .expect("duplicate ingress publication");
            if count == 2 {
                break count;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("duplicate ingress settles");
    assert_eq!(duplicate_count, 2);
    assert_eq!(mailbox_counts(pool, gateway).await, (2, 2, 2));

    CookingCheckpoint { alice, bob }
}

// The follow-up retains the full provenance and approval assertions in one
// reviewable post-restart slice.
