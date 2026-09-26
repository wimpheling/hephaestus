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

#[path = "epoch.rs"]
mod epoch;
#[path = "identity.rs"]
mod identity;
#[path = "live.rs"]
mod live;
#[path = "recovery.rs"]
mod recovery;
