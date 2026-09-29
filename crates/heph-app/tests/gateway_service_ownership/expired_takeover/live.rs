use super::*;

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn exact_takeover_rejects_live_claim() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let fixture = seed_fixture(&pool, "http.service.v1").await;
    let ownership = worker_ownership().await;
    let owner = GatewayServiceOwner::new("exact-takeover-host", Uuid::new_v4()).expect("owner");
    let live = ownership
        .claim_new(
            fixture.gateway,
            fixture.revision,
            &owner,
            Duration::from_secs(30),
        )
        .await
        .expect("live claim");
    let new_owner =
        GatewayServiceOwner::new("exact-takeover-host", Uuid::new_v4()).expect("new owner");
    assert!(matches!(
        ownership
            .claim_expired_instance(&live, &new_owner, Duration::from_secs(30))
            .await,
        Err(GatewayServiceOwnershipError::StaleLease)
    ));
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn exact_takeover_requires_same_host_and_full_identity() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let fixture = seed_fixture(&pool, "http.service.v1").await;
    let ownership = worker_ownership().await;
    let owner = GatewayServiceOwner::new("exact-scope-host", Uuid::new_v4()).expect("owner");
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
    let foreign = GatewayServiceOwner::new("different-host", Uuid::new_v4()).expect("foreign");
    assert!(matches!(
        ownership
            .claim_expired_instance(&claim, &foreign, Duration::from_secs(30))
            .await,
        Err(GatewayServiceOwnershipError::StaleLease)
    ));
    let wrong_revision = GatewayServiceIdentity {
        instance_id: claim.identity.instance_id,
        gateway_id: claim.identity.gateway_id,
        revision_id: Uuid::new_v4(),
    };
    assert!(matches!(
        ownership
            .claim_expired_instance(
                &GatewayServiceInstanceLease {
                    identity: wrong_revision,
                    ..claim.clone()
                },
                &owner,
                Duration::from_secs(30),
            )
            .await,
        Err(GatewayServiceOwnershipError::StaleLease)
    ));
}
