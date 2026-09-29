use super::*;

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn ownership_promotion_rechecks_expiry_after_instance_lock() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let fixture = seed_fixture(&pool, "http.service.v1").await;
    let candidate = seed_service_revision(&pool, fixture.gateway).await;
    sqlx::query("UPDATE gateways SET desired_service_revision_id = $2 WHERE id = $1")
        .bind(fixture.gateway)
        .bind(candidate)
        .execute(&pool)
        .await
        .expect("desired candidate");
    let ownership = worker_ownership().await;
    let owner = GatewayServiceOwner::new("promotion-lock-host", Uuid::new_v4()).expect("owner");
    let claim = ownership
        .claim_new(fixture.gateway, candidate, &owner, Duration::from_secs(1))
        .await
        .expect("candidate claim");
    let ready = ownership
        .mark_ready(
            &ownership
                .mark_starting(&claim, &owner)
                .await
                .expect("candidate starting"),
            &owner,
        )
        .await
        .expect("candidate ready");
    let mut blocker = pool.begin().await.expect("blocker transaction");
    sqlx::query("SELECT id FROM gateway_service_instances WHERE id = $1 FOR UPDATE")
        .bind(ready.identity.instance_id)
        .execute(&mut *blocker)
        .await
        .expect("lock candidate instance");
    let promotion = tokio::spawn({
        let ownership = ownership.clone();
        let ready = ready.clone();
        let owner = owner.clone();
        async move { ownership.promote_ready(&ready, &owner).await }
    });
    wait_for_lock(&pool, "gateway_service_instances").await;
    tokio::time::sleep(Duration::from_millis(1_100)).await;
    blocker.commit().await.expect("release candidate lock");
    assert!(matches!(
        promotion.await.expect("promotion task"),
        Err(GatewayServiceOwnershipError::StaleLease)
    ));
    let active: Option<Uuid> =
        sqlx::query_scalar("SELECT active_revision_id FROM gateways WHERE id = $1")
            .bind(fixture.gateway)
            .fetch_one(&pool)
            .await
            .expect("active pointer");
    assert_eq!(active, Some(fixture.revision));
}
