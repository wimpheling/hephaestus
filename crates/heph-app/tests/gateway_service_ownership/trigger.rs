use super::*;

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn ownership_sql_trigger_rejects_identity_and_fence_bypass() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let fixture = seed_fixture(&pool, "http.service.v1").await;
    let ownership = worker_ownership().await;
    let owner = GatewayServiceOwner::new("trigger-host", Uuid::new_v4()).expect("owner");
    let claim = ownership
        .claim_new(
            fixture.gateway,
            fixture.revision,
            &owner,
            Duration::from_secs(30),
        )
        .await
        .expect("claim");
    let identity_error = sqlx::query(
        "UPDATE gateway_service_instances
            SET gateway_id = $2
          WHERE id = $1",
    )
    .bind(claim.identity.instance_id)
    .bind(Uuid::new_v4())
    .execute(&pool)
    .await;
    assert!(identity_error.is_err());
    let fence_error = sqlx::query(
        "UPDATE gateway_service_instances
            SET fencing_token = fencing_token + 1
          WHERE id = $1",
    )
    .bind(claim.identity.instance_id)
    .execute(&pool)
    .await;
    assert!(fence_error.is_err());
    let transition_error = sqlx::query(
        "UPDATE gateway_service_instances
            SET state = 'ready'
          WHERE id = $1",
    )
    .bind(claim.identity.instance_id)
    .execute(&pool)
    .await;
    assert!(transition_error.is_err());
}
