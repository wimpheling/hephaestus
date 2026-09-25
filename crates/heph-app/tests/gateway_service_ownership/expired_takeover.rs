//! Exact-instance expired takeover coverage for boot recovery.

use super::{seed_fixture, test_pool, wait_for_lock_named, worker_ownership};
use gateway_domain::{
    GatewayServiceExpiredClaimRecovery, GatewayServiceIdentity, GatewayServiceInstanceLease,
    GatewayServiceInstanceState, GatewayServiceOwner, GatewayServiceOwnership,
    GatewayServiceOwnershipError,
};
use serial_test::serial;
use sqlx::PgPool;
use std::{sync::Arc, time::Duration};
use time::{Duration as TimeDuration, OffsetDateTime};
use tokio::time::{sleep, timeout};
use uuid::Uuid;

async fn wait_until_expired(pool: &PgPool, instance_id: Uuid) {
    timeout(Duration::from_secs(2), async {
        loop {
            let expired: bool = sqlx::query_scalar(
                "SELECT lease_expires_at <= clock_timestamp()
                   FROM gateway_service_instances
                  WHERE id = $1",
            )
            .bind(instance_id)
            .fetch_one(pool)
            .await
            .expect("observe exact claim expiry");
            if expired {
                return;
            }
            sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("database confirms exact claim expiry");
}

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

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn exact_resolution_includes_cleaned_and_rejects_wrong_identity() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let fixture = seed_fixture(&pool, "http.service.v1").await;
    let ownership = worker_ownership().await;
    let owner = GatewayServiceOwner::new("exact-resolution-host", Uuid::new_v4()).expect("owner");
    let claim = ownership
        .claim_new(
            fixture.gateway,
            fixture.revision,
            &owner,
            Duration::from_secs(30),
        )
        .await
        .expect("claim");
    let resolved = ownership
        .resolve_exact_instance(claim.identity)
        .await
        .expect("resolve exact claim")
        .expect("exact claim exists");
    assert_eq!(resolved.identity, claim.identity);
    assert_eq!(resolved.fencing_token, claim.fencing_token);

    assert!(
        ownership
            .resolve_exact_instance(GatewayServiceIdentity {
                revision_id: Uuid::new_v4(),
                ..claim.identity
            })
            .await
            .expect("wrong revision is an exact absence")
            .is_none()
    );
    assert!(
        ownership
            .resolve_exact_instance(GatewayServiceIdentity {
                instance_id: Uuid::new_v4(),
                ..claim.identity
            })
            .await
            .expect("missing instance is an exact absence")
            .is_none()
    );
    assert!(matches!(
        ownership
            .resolve_exact_instance(GatewayServiceIdentity {
                instance_id: Uuid::nil(),
                ..claim.identity
            })
            .await,
        Err(GatewayServiceOwnershipError::InvalidArgument)
    ));

    ownership
        .mark_stopping(&claim, &owner)
        .await
        .expect("mark stopping");
    ownership
        .mark_cleaned(&claim, &owner)
        .await
        .expect("mark cleaned");
    let cleaned = ownership
        .resolve_exact_instance(claim.identity)
        .await
        .expect("resolve cleaned claim")
        .expect("cleaned exact row remains queryable");
    assert_eq!(cleaned.state, GatewayServiceInstanceState::Cleaned);
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn exact_resolution_serializes_committed_and_rolled_back_epochs() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let fixture = seed_fixture(&pool, "http.service.v1").await;
    let ownership = Arc::new(worker_ownership().await);
    let old_owner =
        GatewayServiceOwner::new("exact-resolution-barrier", Uuid::new_v4()).expect("old owner");
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
    let recovery_owner = GatewayServiceOwner::new("exact-resolution-barrier", Uuid::new_v4())
        .expect("recovery owner");

    let mut holder = pool.begin().await.expect("begin gateway barrier");
    sqlx::query("SET application_name = 'gateway-exact-resolution-holder'")
        .execute(&mut *holder)
        .await
        .expect("name gateway barrier");
    let holder_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *holder)
        .await
        .expect("gateway barrier pid");
    sqlx::query("SELECT id FROM gateways WHERE id = $1 FOR UPDATE")
        .bind(fixture.gateway)
        .execute(&mut *holder)
        .await
        .expect("hold gateway barrier");
    sqlx::query(
        "UPDATE gateway_service_instances
            SET owner_uuid = $2, fencing_token = $3, state = 'stopping',
                heartbeat_at = clock_timestamp(),
                lease_expires_at = clock_timestamp() + interval '1 millisecond'
          WHERE id = $1",
    )
    .bind(claim.identity.instance_id)
    .bind(recovery_owner.owner_uuid)
    .bind(claim.fencing_token + 1)
    .execute(&mut *holder)
    .await
    .expect("stage committed takeover");

    let resolving = tokio::spawn({
        let ownership = Arc::clone(&ownership);
        let identity = claim.identity;
        async move { ownership.resolve_exact_instance(identity).await }
    });
    wait_for_lock_named(&pool, "gateway-ownership-test", "FROM gateways").await;
    let blocked_by: Vec<i32> = sqlx::query_scalar(
        "SELECT unnest(pg_blocking_pids(pid))
           FROM pg_stat_activity
          WHERE application_name = 'gateway-ownership-test'
            AND state = 'active'
            AND wait_event_type = 'Lock'
            AND query LIKE '%FROM gateways%'
          LIMIT 1",
    )
    .fetch_all(&pool)
    .await
    .expect("inspect exact resolution blocker pid");
    assert!(
        blocked_by.contains(&holder_pid),
        "exact resolver was not blocked by gateway holder {holder_pid}: {blocked_by:?}"
    );
    holder.commit().await.expect("commit staged takeover");
    let committed = timeout(Duration::from_secs(2), resolving)
        .await
        .expect("committed exact resolution completion")
        .expect("committed exact resolution task")
        .expect("resolve committed epoch")
        .expect("committed takeover exists");
    assert_eq!(committed.fencing_token, claim.fencing_token + 1);
    assert_eq!(committed.owner_uuid, recovery_owner.owner_uuid);

    wait_until_expired(&pool, claim.identity.instance_id).await;
    let mut rollback_holder = pool.begin().await.expect("begin rollback barrier");
    sqlx::query("SET application_name = 'gateway-exact-resolution-holder'")
        .execute(&mut *rollback_holder)
        .await
        .expect("name rollback barrier");
    let rollback_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *rollback_holder)
        .await
        .expect("rollback barrier pid");
    sqlx::query("SELECT id FROM gateways WHERE id = $1 FOR UPDATE")
        .bind(fixture.gateway)
        .execute(&mut *rollback_holder)
        .await
        .expect("hold rollback gateway barrier");
    let rolled_owner =
        GatewayServiceOwner::new("exact-resolution-barrier", Uuid::new_v4()).expect("rolled owner");
    sqlx::query(
        "UPDATE gateway_service_instances
            SET owner_uuid = $2, fencing_token = $3, state = 'stopping',
                heartbeat_at = clock_timestamp(),
                lease_expires_at = clock_timestamp() + interval '30 seconds'
          WHERE id = $1",
    )
    .bind(claim.identity.instance_id)
    .bind(rolled_owner.owner_uuid)
    .bind(committed.fencing_token + 1)
    .execute(&mut *rollback_holder)
    .await
    .expect("stage rolled-back takeover");
    let resolving = tokio::spawn({
        let ownership = Arc::clone(&ownership);
        let identity = claim.identity;
        async move { ownership.resolve_exact_instance(identity).await }
    });
    wait_for_lock_named(&pool, "gateway-ownership-test", "FROM gateways").await;
    let rollback_blocked_by: Vec<i32> = sqlx::query_scalar(
        "SELECT unnest(pg_blocking_pids(pid))
           FROM pg_stat_activity
          WHERE application_name = 'gateway-ownership-test'
            AND state = 'active'
            AND wait_event_type = 'Lock'
            AND query LIKE '%FROM gateways%'
          LIMIT 1",
    )
    .fetch_all(&pool)
    .await
    .expect("inspect rollback resolution blocker pid");
    assert!(
        rollback_blocked_by.contains(&rollback_pid),
        "exact resolver was not blocked by rollback holder {rollback_pid}: {rollback_blocked_by:?}"
    );
    rollback_holder
        .rollback()
        .await
        .expect("rollback staged newer epoch");
    let resolved = timeout(Duration::from_secs(2), resolving)
        .await
        .expect("exact resolution completion")
        .expect("exact resolution task")
        .expect("resolve rolled-back epoch")
        .expect("original committed epoch remains");
    assert_eq!(resolved.fencing_token, committed.fencing_token);
    assert_eq!(resolved.owner_uuid, recovery_owner.owner_uuid);
}
