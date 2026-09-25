use super::*;

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn ownership_old_active_drains_only_after_candidate_promotion() {
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
    let owner = GatewayServiceOwner::new("cutover-host", Uuid::new_v4()).expect("owner");
    let old = ownership
        .claim_new(
            fixture.gateway,
            fixture.revision,
            &owner,
            Duration::from_secs(30),
        )
        .await
        .expect("old claim");
    let old = ownership
        .mark_ready(
            &ownership
                .mark_starting(&old, &owner)
                .await
                .expect("old starting"),
            &owner,
        )
        .await
        .expect("old ready");
    let candidate_claim = ownership
        .claim_new(fixture.gateway, candidate, &owner, Duration::from_secs(30))
        .await
        .expect("candidate claim");
    let candidate_ready = ownership
        .mark_ready(
            &ownership
                .mark_starting(&candidate_claim, &owner)
                .await
                .expect("candidate starting"),
            &owner,
        )
        .await
        .expect("candidate ready");
    assert!(matches!(
        ownership.mark_draining(&old, &owner).await,
        Err(GatewayServiceOwnershipError::Conflict)
    ));
    assert_eq!(
        ownership
            .promote_ready(&candidate_ready, &owner)
            .await
            .expect("candidate cutover"),
        Some(fixture.revision)
    );
    ownership
        .mark_draining(&old, &owner)
        .await
        .expect("old revision drain after cutover");
}
