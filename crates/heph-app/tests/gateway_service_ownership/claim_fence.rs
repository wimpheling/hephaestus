use super::*;

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn ownership_claims_one_live_service_and_fences_stale_owner() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let fixture = seed_fixture(&pool, "http.service.v1").await;
    let ownership = Arc::new(worker_ownership().await);
    let first = GatewayServiceOwner::new("ownership-host", Uuid::new_v4()).expect("owner");
    let second = GatewayServiceOwner::new("ownership-host", Uuid::new_v4()).expect("owner");
    let (left, right) = tokio::join!(
        ownership.claim_new(
            fixture.gateway,
            fixture.revision,
            &first,
            Duration::from_millis(1),
        ),
        ownership.claim_new(
            fixture.gateway,
            fixture.revision,
            &second,
            Duration::from_millis(1),
        )
    );
    let (first_claim, old_owner) = match (left, right) {
        (Ok(claim), Err(GatewayServiceOwnershipError::Conflict)) => (claim, first),
        (Err(GatewayServiceOwnershipError::Conflict), Ok(claim)) => (claim, second),
        _ => panic!("concurrent claim did not produce one winner"),
    };
    assert_eq!(first_claim.state, GatewayServiceInstanceState::Provisioning);
    assert_eq!(
        first_claim.vm_id,
        format!("gateway-service-{}", first_claim.identity.instance_id)
    );

    tokio::time::sleep(Duration::from_millis(10)).await;
    let recovery_owner = old_owner.clone();
    let recovered = ownership
        .claim_expired(&recovery_owner, Duration::from_secs(30), 1)
        .await
        .expect("recover claim");
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].fencing_token, first_claim.fencing_token + 1);
    assert_eq!(recovered[0].state, GatewayServiceInstanceState::Stopping);
    assert_eq!(recovered[0].owner_uuid, old_owner.owner_uuid);
    assert!(matches!(
        ownership
            .renew(&first_claim, &old_owner, Duration::from_secs(30))
            .await,
        Err(GatewayServiceOwnershipError::StaleLease)
    ));
    ownership
        .mark_cleaned(&recovered[0], &recovery_owner)
        .await
        .expect("clean recovered claim");
}
