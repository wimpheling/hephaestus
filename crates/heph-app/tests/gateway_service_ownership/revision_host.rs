use super::*;

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn ownership_requires_exact_service_revision_and_host() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let service = seed_fixture(&pool, "http.service.v1").await;
    sqlx::query(
        "UPDATE gateways
            SET active_revision_id = NULL, desired_service_revision_id = $2
          WHERE id = $1",
    )
    .bind(service.gateway)
    .bind(service.revision)
    .execute(&pool)
    .await
    .expect("pending service revision");
    let stateless = seed_fixture(&pool, "http.v1").await;
    let ownership = worker_ownership().await;
    let owner = GatewayServiceOwner::new("ownership-host", Uuid::new_v4()).expect("owner");
    let mut invalid_owner = owner.clone();
    invalid_owner.host_id = "ownership host".to_owned();
    assert!(matches!(
        ownership
            .claim_new(
                service.gateway,
                service.revision,
                &invalid_owner,
                Duration::from_secs(30),
            )
            .await,
        Err(GatewayServiceOwnershipError::InvalidArgument)
    ));
    assert!(matches!(
        ownership
            .claim_new(
                Uuid::nil(),
                service.revision,
                &owner,
                Duration::from_secs(30),
            )
            .await,
        Err(GatewayServiceOwnershipError::InvalidArgument)
    ));
    assert!(matches!(
        ownership
            .claim_new(
                stateless.gateway,
                stateless.revision,
                &owner,
                Duration::from_secs(30),
            )
            .await,
        Err(GatewayServiceOwnershipError::Conflict)
    ));
    ownership
        .claim_new(
            service.gateway,
            service.revision,
            &owner,
            Duration::from_millis(1),
        )
        .await
        .expect("service claim");

    let active_and_pending = seed_fixture(&pool, "http.service.v1").await;
    let pending_revision = seed_service_revision(&pool, active_and_pending.gateway).await;
    sqlx::query("UPDATE gateways SET desired_service_revision_id = $2 WHERE id = $1")
        .bind(active_and_pending.gateway)
        .bind(pending_revision)
        .execute(&pool)
        .await
        .expect("pending replacement revision");
    ownership
        .claim_new(
            active_and_pending.gateway,
            active_and_pending.revision,
            &owner,
            Duration::from_secs(30),
        )
        .await
        .expect("active service remains claimable during pending replacement");
    ownership
        .claim_new(
            active_and_pending.gateway,
            pending_revision,
            &owner,
            Duration::from_secs(30),
        )
        .await
        .expect("pending service replacement claim");
    tokio::time::sleep(Duration::from_millis(10)).await;
    let other_host = GatewayServiceOwner::new("other-host", Uuid::new_v4()).expect("owner");
    assert!(
        ownership
            .claim_expired(&other_host, Duration::from_secs(30), 1)
            .await
            .expect("other-host recovery")
            .is_empty()
    );
}
