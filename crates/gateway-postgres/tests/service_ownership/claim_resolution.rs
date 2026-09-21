//! Real `PostgreSQL` claim-resolution barrier coverage.

use super::{seed_fixture, test_pool, wait_for_lock_named, worker_ownership};
use gateway_edge::{
    GatewayServiceClaimResolutionStore, GatewayServiceInstanceState, GatewayServiceOwner,
    GatewayServiceOwnership,
};
use serial_test::serial;
use sqlx::PgPool;
use std::{sync::Arc, time::Duration};
use tokio::{
    sync::oneshot,
    time::{sleep, timeout},
};
use uuid::Uuid;

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn claim_resolution_waits_for_commit_then_returns_uncommitted_claim() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let fixture = seed_fixture(&pool, "http.service.v1").await;
    let ownership = Arc::new(worker_ownership().await);
    let barrier = start_uncommitted_claim(&pool, fixture.gateway, fixture.revision).await;
    let resolving = tokio::spawn({
        let ownership = Arc::clone(&ownership);
        async move {
            ownership
                .resolve_revision_claim(fixture.gateway, fixture.revision)
                .await
        }
    });
    wait_for_lock_named(&pool, "gateway-ownership-test", "FROM gateways").await;
    barrier.release.send(true).expect("commit barrier");
    barrier.task.await.expect("barrier task");
    let resolved = timeout(Duration::from_secs(2), resolving)
        .await
        .expect("resolver completion")
        .expect("resolver task")
        .expect("resolve committed claim")
        .expect("committed claim");
    assert_eq!(resolved.identity.instance_id, barrier.instance_id);
    assert_eq!(resolved.state, GatewayServiceInstanceState::Provisioning);
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn claim_resolution_waits_for_rollback_then_returns_none() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let fixture = seed_fixture(&pool, "http.service.v1").await;
    let ownership = Arc::new(worker_ownership().await);
    let barrier = start_uncommitted_claim(&pool, fixture.gateway, fixture.revision).await;
    let resolving = tokio::spawn({
        let ownership = Arc::clone(&ownership);
        async move {
            ownership
                .resolve_revision_claim(fixture.gateway, fixture.revision)
                .await
        }
    });
    wait_for_lock_named(&pool, "gateway-ownership-test", "FROM gateways").await;
    barrier.release.send(false).expect("rollback barrier");
    barrier.task.await.expect("barrier task");
    assert!(
        timeout(Duration::from_secs(2), resolving)
            .await
            .expect("resolver completion")
            .expect("resolver task")
            .expect("resolve rolled-back claim")
            .is_none()
    );
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn claim_resolution_exposes_expired_new_fence_and_excludes_cleaned() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let fixture = seed_fixture(&pool, "http.service.v1").await;
    let ownership = worker_ownership().await;
    let host_id = format!("claim-resolution-host-{}", Uuid::new_v4());
    let first_owner =
        GatewayServiceOwner::new(host_id.clone(), Uuid::new_v4()).expect("first owner");
    let first = ownership
        .claim_new(
            fixture.gateway,
            fixture.revision,
            &first_owner,
            Duration::from_millis(1),
        )
        .await
        .expect("first claim");
    timeout(Duration::from_secs(2), async {
        loop {
            let expired: bool = sqlx::query_scalar(
                "SELECT lease_expires_at <= clock_timestamp()
                   FROM gateway_service_instances
                  WHERE id = $1",
            )
            .bind(first.identity.instance_id)
            .fetch_one(&pool)
            .await
            .expect("observe expired claim");
            if expired {
                break;
            }
            sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("database confirms claim expiry");
    let expired = ownership
        .resolve_revision_claim(fixture.gateway, fixture.revision)
        .await
        .expect("resolve expired claim")
        .expect("expired claim remains visible before recovery");
    assert_eq!(expired.identity.instance_id, first.identity.instance_id);
    assert_eq!(expired.fencing_token, first.fencing_token);
    assert_eq!(expired.owner_uuid, first_owner.owner_uuid);
    assert_eq!(expired.owner_host_id, host_id);
    let second_owner = GatewayServiceOwner::new(host_id, Uuid::new_v4()).expect("recovery owner");
    let recovered = ownership
        .claim_expired(&second_owner, Duration::from_secs(30), 1)
        .await
        .expect("recover expired claim");
    assert_eq!(recovered.len(), 1);
    let resolved = ownership
        .resolve_revision_claim(fixture.gateway, fixture.revision)
        .await
        .expect("resolve recovered claim")
        .expect("recovered claim");
    assert_eq!(resolved.identity.instance_id, first.identity.instance_id);
    assert_eq!(resolved.fencing_token, first.fencing_token + 1);
    assert_eq!(resolved.owner_uuid, second_owner.owner_uuid);
    assert!(resolved.lease_expires_at > first.lease_expires_at);
    assert_eq!(resolved.state, GatewayServiceInstanceState::Stopping);
    ownership
        .mark_cleaned(&resolved, &second_owner)
        .await
        .expect("clean recovered claim");
    assert!(
        ownership
            .resolve_revision_claim(fixture.gateway, fixture.revision)
            .await
            .expect("resolve cleaned claim")
            .is_none()
    );
}

#[tokio::test]
#[serial]
async fn claim_resolution_rejects_nil_identity_without_database_access() {
    let Some(_pool) = test_pool().await else {
        return;
    };
    let ownership = worker_ownership().await;
    assert!(matches!(
        ownership
            .resolve_revision_claim(Uuid::nil(), Uuid::new_v4())
            .await,
        Err(gateway_edge::GatewayServiceOwnershipError::InvalidArgument)
    ));
    assert!(matches!(
        ownership
            .resolve_revision_claim(Uuid::new_v4(), Uuid::nil())
            .await,
        Err(gateway_edge::GatewayServiceOwnershipError::InvalidArgument)
    ));
    assert!(
        ownership
            .resolve_revision_claim(Uuid::new_v4(), Uuid::new_v4())
            .await
            .expect("missing gateway is a resolved absence")
            .is_none()
    );
}

struct ClaimBarrier {
    instance_id: Uuid,
    release: oneshot::Sender<bool>,
    task: tokio::task::JoinHandle<()>,
}

async fn start_uncommitted_claim(
    pool: &PgPool,
    gateway_id: Uuid,
    revision_id: Uuid,
) -> ClaimBarrier {
    let instance_id = Uuid::new_v4();
    let (ready, ready_rx) = oneshot::channel();
    let (release, release_rx) = oneshot::channel();
    let pool = pool.clone();
    let task = tokio::spawn(async move {
        let mut transaction = pool.begin().await.expect("claim barrier transaction");
        sqlx::query("SELECT id FROM gateways WHERE id = $1 FOR UPDATE")
            .bind(gateway_id)
            .execute(&mut *transaction)
            .await
            .expect("lock gateway row");
        sqlx::query(
            "INSERT INTO gateway_service_instances
                (id, gateway_id, revision_id, owner_host_id, owner_uuid,
                 fencing_token, vm_id, state, lease_expires_at, heartbeat_at)
             VALUES ($1, $2, $3, 'claim-barrier-host', $4, 1,
                     $5, 'provisioning', clock_timestamp() + interval '30 seconds',
                     clock_timestamp())",
        )
        .bind(instance_id)
        .bind(gateway_id)
        .bind(revision_id)
        .bind(Uuid::new_v4())
        .bind(format!("gateway-service-{instance_id}"))
        .execute(&mut *transaction)
        .await
        .expect("insert uncommitted claim");
        ready.send(()).expect("barrier ready receiver");
        if release_rx.await.expect("barrier release") {
            transaction.commit().await.expect("commit claim barrier");
        } else {
            transaction
                .rollback()
                .await
                .expect("rollback claim barrier");
        }
    });
    ready_rx.await.expect("claim barrier ready");
    ClaimBarrier {
        instance_id,
        release,
        task,
    }
}
