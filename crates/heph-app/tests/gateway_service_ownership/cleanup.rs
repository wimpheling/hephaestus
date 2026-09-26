use super::*;

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn ownership_cleanup_failure_keeps_live_uniqueness_and_batch_is_bounded() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let ownership = worker_ownership().await;
    let owner = GatewayServiceOwner::new("batch-host", Uuid::new_v4()).expect("owner");
    let mut fixtures = Vec::with_capacity(MAX_SERVICE_OWNERSHIP_BATCH + 2);
    for _ in 0..MAX_SERVICE_OWNERSHIP_BATCH + 2 {
        fixtures.push(seed_fixture(&pool, "http.service.v1").await);
    }
    let mut claims = Vec::with_capacity(fixtures.len());
    for fixture in &fixtures {
        claims.push(
            ownership
                .claim_new(
                    fixture.gateway,
                    fixture.revision,
                    &owner,
                    Duration::from_millis(1),
                )
                .await
                .expect("batch claim"),
        );
    }
    tokio::time::sleep(Duration::from_millis(10)).await;
    let recovery_owner = GatewayServiceOwner::new("batch-host", Uuid::new_v4()).expect("owner");
    let first = ownership
        .claim_expired(
            &recovery_owner,
            Duration::from_secs(30),
            MAX_SERVICE_OWNERSHIP_BATCH,
        )
        .await
        .expect("first batch");
    assert_eq!(first.len(), MAX_SERVICE_OWNERSHIP_BATCH);
    let second = ownership
        .claim_expired(
            &recovery_owner,
            Duration::from_secs(30),
            MAX_SERVICE_OWNERSHIP_BATCH,
        )
        .await
        .expect("second batch");
    assert_eq!(second.len(), 2);

    let first_recovered = first.first().expect("recovered claim");
    let cleanup_claim = ownership
        .claim_new(
            first_recovered.identity.gateway_id,
            first_recovered.identity.revision_id,
            &owner,
            Duration::from_secs(30),
        )
        .await;
    assert!(matches!(
        cleanup_claim,
        Err(GatewayServiceOwnershipError::Conflict)
    ));
    ownership
        .mark_cleaned(first_recovered, &recovery_owner)
        .await
        .expect("mark cleaned");
    ownership
        .claim_new(
            first_recovered.identity.gateway_id,
            first_recovered.identity.revision_id,
            &owner,
            Duration::from_secs(30),
        )
        .await
        .expect("claim after cleanup");
    drop(claims);
}
