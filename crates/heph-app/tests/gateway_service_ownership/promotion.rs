use super::*;

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn ownership_transitions_and_promotes_pending_service() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let fixture = seed_fixture(&pool, "http.service.v1").await;
    let candidate = seed_service_revision(&pool, fixture.gateway).await;
    sqlx::query(
        "UPDATE gateways
            SET active_revision_id = NULL, desired_service_revision_id = $2
          WHERE id = $1",
    )
    .bind(fixture.gateway)
    .bind(candidate)
    .execute(&pool)
    .await
    .expect("pending candidate pointers");
    let before_events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM application_events
          WHERE aggregate_type = 'gateway' AND aggregate_id = $1",
    )
    .bind(fixture.gateway)
    .fetch_one(&pool)
    .await
    .expect("promotion event baseline");
    let ownership = worker_ownership().await;
    let owner = GatewayServiceOwner::new("promotion-host", Uuid::new_v4()).expect("owner");
    let claim = ownership
        .claim_new(fixture.gateway, candidate, &owner, Duration::from_secs(30))
        .await
        .expect("candidate claim");
    assert!(matches!(
        ownership.mark_ready(&claim, &owner).await,
        Err(GatewayServiceOwnershipError::Conflict)
    ));
    let starting = ownership
        .mark_starting(&claim, &owner)
        .await
        .expect("starting transition");
    assert!(matches!(
        ownership.mark_starting(&starting, &owner).await,
        Err(GatewayServiceOwnershipError::Conflict)
    ));
    let ready = ownership
        .mark_ready(&starting, &owner)
        .await
        .expect("ready transition");
    assert_eq!(ready.state, GatewayServiceInstanceState::Ready);
    let previous = ownership
        .promote_ready(&ready, &owner)
        .await
        .expect("promote initial candidate");
    assert_eq!(previous, None);
    let pointers: (Option<Uuid>, Option<Uuid>) = sqlx::query_as(
        "SELECT active_revision_id, desired_service_revision_id
           FROM gateways WHERE id = $1",
    )
    .bind(fixture.gateway)
    .fetch_one(&pool)
    .await
    .expect("promoted pointers");
    assert_eq!(pointers, (Some(candidate), Some(candidate)));
    let after_events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM application_events
          WHERE aggregate_type = 'gateway' AND aggregate_id = $1",
    )
    .bind(fixture.gateway)
    .fetch_one(&pool)
    .await
    .expect("promotion event count");
    assert_eq!(after_events, before_events + 1);
    assert_eq!(
        ownership
            .promote_ready(&ready, &owner)
            .await
            .expect("idempotent promotion retry"),
        Some(candidate)
    );
    let retry_events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM application_events
          WHERE aggregate_type = 'gateway' AND aggregate_id = $1",
    )
    .bind(fixture.gateway)
    .fetch_one(&pool)
    .await
    .expect("idempotent event count");
    assert_eq!(retry_events, after_events);
}
