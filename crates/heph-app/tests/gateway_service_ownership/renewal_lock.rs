use super::*;

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn ownership_renewal_checks_expiry_after_waiting_for_instance_lock() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let fixture = seed_fixture(&pool, "http.service.v1").await;
    let ownership = worker_ownership().await;
    let owner = GatewayServiceOwner::new("lock-host", Uuid::new_v4()).expect("owner");
    let claim = ownership
        .claim_new(
            fixture.gateway,
            fixture.revision,
            &owner,
            Duration::from_millis(30),
        )
        .await
        .expect("claim");
    let mut blocker = pool.begin().await.expect("blocker transaction");
    sqlx::query("SELECT id FROM gateway_service_instances WHERE id = $1 FOR UPDATE")
        .bind(claim.identity.instance_id)
        .execute(&mut *blocker)
        .await
        .expect("lock instance");
    let renewal = tokio::spawn({
        let ownership = ownership.clone();
        let claim = claim.clone();
        let owner = owner.clone();
        async move {
            ownership
                .renew(&claim, &owner, Duration::from_secs(30))
                .await
        }
    });
    wait_for_lock(&pool, "gateway_service_instances").await;
    tokio::time::sleep(Duration::from_millis(60)).await;
    blocker.commit().await.expect("release instance lock");
    assert!(matches!(
        renewal.await.expect("renewal task"),
        Err(GatewayServiceOwnershipError::StaleLease)
    ));
}
