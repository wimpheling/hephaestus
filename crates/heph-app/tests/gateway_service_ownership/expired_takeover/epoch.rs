use super::*;

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn exact_takeover_fences_expired_claim_and_rejects_cleaned_row() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let fixture = seed_fixture(&pool, "http.service.v1").await;
    let ownership = worker_ownership().await;
    let old_owner = GatewayServiceOwner::new("fence-host", Uuid::new_v4()).expect("owner");
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
    let new_owner = GatewayServiceOwner::new("fence-host", Uuid::new_v4()).expect("new owner");
    let recovered = ownership
        .claim_expired_instance(&claim, &new_owner, Duration::from_secs(30))
        .await
        .expect("exact expired takeover");
    assert_eq!(recovered.identity, claim.identity);
    assert_eq!(recovered.vm_id, claim.vm_id);
    assert_eq!(recovered.fencing_token, claim.fencing_token + 1);
    assert_eq!(recovered.owner_host_id, new_owner.host_id);
    assert_eq!(recovered.owner_uuid, new_owner.owner_uuid);
    assert_eq!(recovered.state, GatewayServiceInstanceState::Stopping);
    assert!(recovered.lease_expires_at > recovered.heartbeat_at);
    ownership
        .mark_cleaned(&recovered, &new_owner)
        .await
        .expect("clean recovered claim");
    let cleaned_id = Uuid::new_v4();
    let heartbeat_at = OffsetDateTime::now_utc() - TimeDuration::seconds(2);
    let lease_expires_at = heartbeat_at + TimeDuration::seconds(1);
    let cleaned_at = OffsetDateTime::now_utc();
    sqlx::query(
        "INSERT INTO gateway_service_instances
            (id, gateway_id, revision_id, owner_host_id, owner_uuid,
             fencing_token, vm_id, state, lease_expires_at, heartbeat_at, cleaned_at)
         VALUES ($1, $2, $3, $4, $5, 1, $6, 'cleaned', $7, $8, $9)",
    )
    .bind(cleaned_id)
    .bind(recovered.identity.gateway_id)
    .bind(recovered.identity.revision_id)
    .bind(&new_owner.host_id)
    .bind(new_owner.owner_uuid)
    .bind(format!("gateway-service-{cleaned_id}"))
    .bind(lease_expires_at)
    .bind(heartbeat_at)
    .bind(cleaned_at)
    .execute(&pool)
    .await
    .expect("insert expired cleaned row fixture");
    assert!(matches!(
        ownership
            .claim_expired_instance(
                &GatewayServiceInstanceLease {
                    identity: GatewayServiceIdentity {
                        instance_id: cleaned_id,
                        gateway_id: recovered.identity.gateway_id,
                        revision_id: recovered.identity.revision_id,
                    },
                    owner_host_id: new_owner.host_id.clone(),
                    owner_uuid: new_owner.owner_uuid,
                    fencing_token: 1,
                    state: GatewayServiceInstanceState::Cleaned,
                    vm_id: format!("gateway-service-{cleaned_id}"),
                    lease_expires_at,
                    heartbeat_at,
                },
                &new_owner,
                Duration::from_secs(30),
            )
            .await,
        Err(GatewayServiceOwnershipError::StaleLease)
    ));
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn competing_exact_takeovers_have_one_fenced_winner() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let fixture = seed_fixture(&pool, "http.service.v1").await;
    let ownership = Arc::new(worker_ownership().await);
    let old_owner = GatewayServiceOwner::new("race-host", Uuid::new_v4()).expect("owner");
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
    let left_owner = GatewayServiceOwner::new("race-host", Uuid::new_v4()).expect("left owner");
    let right_owner = GatewayServiceOwner::new("race-host", Uuid::new_v4()).expect("right owner");
    let (left, right) = timeout(Duration::from_secs(2), async {
        tokio::join!(
            ownership.claim_expired_instance(&claim, &left_owner, Duration::from_secs(30)),
            ownership.claim_expired_instance(&claim, &right_owner, Duration::from_secs(30)),
        )
    })
    .await
    .expect("competing exact takeover completion");
    let winner = match (left, right) {
        (Ok(lease), Err(GatewayServiceOwnershipError::StaleLease))
        | (Err(GatewayServiceOwnershipError::StaleLease), Ok(lease)) => lease,
        result => panic!("competing exact takeover result: {result:?}"),
    };
    assert_eq!(winner.fencing_token, claim.fencing_token + 1);
    assert_eq!(winner.state, GatewayServiceInstanceState::Stopping);
}
