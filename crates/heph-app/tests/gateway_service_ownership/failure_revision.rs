use super::*;

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn service_failure_backoff_is_scoped_to_one_revision() {
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
    let failures = worker_failure_store().await;
    let owner = GatewayServiceOwner::new("failure-revision-host", Uuid::new_v4()).expect("owner");
    let old = ownership
        .claim_new(
            fixture.gateway,
            fixture.revision,
            &owner,
            Duration::from_secs(30),
        )
        .await
        .expect("old claim");
    let candidate_claim = ownership
        .claim_new(fixture.gateway, candidate, &owner, Duration::from_secs(30))
        .await
        .expect("candidate claim");
    failures
        .record_failure(
            &old,
            &owner,
            GatewayServiceFailure::new(GatewayServiceFailureCode::Health, None, None)
                .expect("health failure"),
        )
        .await
        .expect("record old revision failure");
    let old_stopping = ownership
        .mark_stopping(&old, &owner)
        .await
        .expect("stop old revision");
    ownership
        .mark_cleaned(&old_stopping, &owner)
        .await
        .expect("clean old revision");
    assert!(matches!(
        ownership
            .claim_new(
                fixture.gateway,
                fixture.revision,
                &owner,
                Duration::from_secs(30),
            )
            .await,
        Err(GatewayServiceOwnershipError::Conflict)
    ));
    let candidate_stopping = ownership
        .mark_stopping(&candidate_claim, &owner)
        .await
        .expect("stop candidate");
    ownership
        .mark_cleaned(&candidate_stopping, &owner)
        .await
        .expect("clean candidate");
    let replacement = ownership
        .claim_new(fixture.gateway, candidate, &owner, Duration::from_secs(30))
        .await
        .expect("candidate revision ignores old backoff");
    let replacement_stopping = ownership
        .mark_stopping(&replacement, &owner)
        .await
        .expect("stop candidate replacement");
    ownership
        .mark_cleaned(&replacement_stopping, &owner)
        .await
        .expect("clean candidate replacement");
}
