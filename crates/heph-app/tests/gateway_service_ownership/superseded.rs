use super::*;

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn ownership_promotion_rejects_superseded_or_revoked_candidates() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let fixture = seed_fixture(&pool, "http.service.v1").await;
    let first_candidate = seed_service_revision(&pool, fixture.gateway).await;
    let second_candidate = seed_service_revision(&pool, fixture.gateway).await;
    sqlx::query("UPDATE gateways SET desired_service_revision_id = $2 WHERE id = $1")
        .bind(fixture.gateway)
        .bind(first_candidate)
        .execute(&pool)
        .await
        .expect("first desired candidate");
    let ownership = worker_ownership().await;
    let owner = GatewayServiceOwner::new("supersede-host", Uuid::new_v4()).expect("owner");
    let first = ownership
        .claim_new(
            fixture.gateway,
            first_candidate,
            &owner,
            Duration::from_secs(30),
        )
        .await
        .expect("first candidate claim");
    let first = ownership
        .mark_ready(
            &ownership
                .mark_starting(&first, &owner)
                .await
                .expect("first candidate starting"),
            &owner,
        )
        .await
        .expect("first candidate ready");
    sqlx::query("UPDATE gateways SET lifecycle = 'paused' WHERE id = $1")
        .bind(fixture.gateway)
        .execute(&pool)
        .await
        .expect("pause gateway");
    assert!(matches!(
        ownership.promote_ready(&first, &owner).await,
        Err(GatewayServiceOwnershipError::Conflict)
    ));
    sqlx::query("UPDATE gateways SET lifecycle = 'enabled' WHERE id = $1")
        .bind(fixture.gateway)
        .execute(&pool)
        .await
        .expect("enable gateway");
    sqlx::query("UPDATE gateways SET desired_service_revision_id = $2 WHERE id = $1")
        .bind(fixture.gateway)
        .bind(second_candidate)
        .execute(&pool)
        .await
        .expect("superseding desired candidate");
    let before_superseded_events: i64 = gateway_event_count(&pool, fixture.gateway).await;
    assert!(matches!(
        ownership.promote_ready(&first, &owner).await,
        Err(GatewayServiceOwnershipError::Conflict)
    ));
    assert_eq!(
        gateway_event_count(&pool, fixture.gateway).await,
        before_superseded_events
    );
    assert_eq!(
        active_pointer(&pool, fixture.gateway).await,
        Some(fixture.revision)
    );

    let second = ownership
        .claim_new(
            fixture.gateway,
            second_candidate,
            &owner,
            Duration::from_secs(30),
        )
        .await
        .expect("second candidate claim");
    let second = ownership
        .mark_ready(
            &ownership
                .mark_starting(&second, &owner)
                .await
                .expect("second candidate starting"),
            &owner,
        )
        .await
        .expect("second candidate ready");
    let release: Uuid =
        sqlx::query_scalar("SELECT release_id FROM gateway_revisions WHERE id = $1")
            .bind(second_candidate)
            .fetch_one(&pool)
            .await
            .expect("candidate release");
    sqlx::query("UPDATE releases SET state = 'revoked', revoked_at = now() WHERE id = $1")
        .bind(release)
        .execute(&pool)
        .await
        .expect("revoke candidate release");
    let before_revoked_events: i64 = gateway_event_count(&pool, fixture.gateway).await;
    assert!(matches!(
        ownership.promote_ready(&second, &owner).await,
        Err(GatewayServiceOwnershipError::Conflict)
    ));
    assert_eq!(
        gateway_event_count(&pool, fixture.gateway).await,
        before_revoked_events
    );
    assert_eq!(
        active_pointer(&pool, fixture.gateway).await,
        Some(fixture.revision)
    );
}
