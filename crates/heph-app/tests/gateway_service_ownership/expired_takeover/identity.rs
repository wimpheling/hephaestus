use super::*;

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn exact_takeover_rejects_prior_epoch_mismatch_without_mutation() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let fixture = seed_fixture(&pool, "http.service.v1").await;
    let ownership = worker_ownership().await;
    let old_owner = GatewayServiceOwner::new("epoch-cas-host", Uuid::new_v4()).expect("owner");
    let claim = ownership
        .claim_new(
            fixture.gateway,
            fixture.revision,
            &old_owner,
            Duration::from_millis(1),
        )
        .await
        .expect("claim");
    wait_until_expired(&pool, claim.identity.instance_id).await;
    let new_owner = GatewayServiceOwner::new("epoch-cas-host", Uuid::new_v4()).expect("new owner");
    let mut wrong_owner_witness = claim.clone();
    wrong_owner_witness.owner_uuid = Uuid::new_v4();
    assert!(matches!(
        ownership
            .claim_expired_instance(&wrong_owner_witness, &new_owner, Duration::from_secs(30))
            .await,
        Err(GatewayServiceOwnershipError::StaleLease)
    ));
    let unchanged: (Uuid, i64, String) = sqlx::query_as(
        "SELECT owner_uuid, fencing_token, state
           FROM gateway_service_instances
          WHERE id = $1",
    )
    .bind(claim.identity.instance_id)
    .fetch_one(&pool)
    .await
    .expect("unchanged row");
    assert_eq!(unchanged.0, old_owner.owner_uuid);
    assert_eq!(unchanged.1, claim.fencing_token);
    assert_eq!(unchanged.2, "provisioning");

    let intermediate =
        GatewayServiceOwner::new("epoch-cas-host", Uuid::new_v4()).expect("intermediate");
    let newer = ownership
        .claim_expired_instance(&claim, &intermediate, Duration::from_millis(1))
        .await
        .expect("first takeover");
    wait_until_expired(&pool, newer.identity.instance_id).await;
    let before_rejected_newer: (Uuid, i64, String) = sqlx::query_as(
        "SELECT owner_uuid, fencing_token, state
           FROM gateway_service_instances
          WHERE id = $1",
    )
    .bind(claim.identity.instance_id)
    .fetch_one(&pool)
    .await
    .expect("newer row");
    let mut stale_fence_witness = newer.clone();
    stale_fence_witness.fencing_token = claim.fencing_token;
    assert!(matches!(
        ownership
            .claim_expired_instance(&stale_fence_witness, &intermediate, Duration::from_secs(30),)
            .await,
        Err(GatewayServiceOwnershipError::StaleLease)
    ));
    assert!(matches!(
        ownership
            .claim_expired_instance(&claim, &new_owner, Duration::from_secs(30))
            .await,
        Err(GatewayServiceOwnershipError::StaleLease)
    ));
    let after_rejected_newer: (Uuid, i64, String) = sqlx::query_as(
        "SELECT owner_uuid, fencing_token, state
           FROM gateway_service_instances
          WHERE id = $1",
    )
    .bind(claim.identity.instance_id)
    .fetch_one(&pool)
    .await
    .expect("unchanged newer row");
    assert_eq!(after_rejected_newer, before_rejected_newer);
    assert_eq!(after_rejected_newer.0, intermediate.owner_uuid);
    assert_eq!(after_rejected_newer.1, claim.fencing_token + 1);
    assert_eq!(after_rejected_newer.2, "stopping");
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn exact_takeover_allows_same_daemon_epoch_recovery() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let fixture = seed_fixture(&pool, "http.service.v1").await;
    let ownership = worker_ownership().await;
    let owner = GatewayServiceOwner::new("same-daemon-host", Uuid::new_v4()).expect("owner");
    let claim = ownership
        .claim_new(
            fixture.gateway,
            fixture.revision,
            &owner,
            Duration::from_millis(1),
        )
        .await
        .expect("claim");
    wait_until_expired(&pool, claim.identity.instance_id).await;
    let recovered = ownership
        .claim_expired_instance(&claim, &owner, Duration::from_secs(30))
        .await
        .expect("same-daemon takeover");
    assert_eq!(recovered.owner_uuid, owner.owner_uuid);
    assert_eq!(recovered.fencing_token, claim.fencing_token + 1);
    assert_eq!(recovered.state, GatewayServiceInstanceState::Stopping);
    ownership
        .mark_cleaned(&recovered, &owner)
        .await
        .expect("clean same-daemon recovery");
}
