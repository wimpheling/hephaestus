//! instance inventory scenario.

use super::support::{seed_gateway_with_instance_state, test_pool, worker_pool};
use gateway_domain::{GatewayServiceInstancePage, GatewayServiceTargetStore};
use gateway_postgres::PostgresGatewayServiceTargets;
use serial_test::serial;
use std::{collections::HashSet, time::Duration};
use time::OffsetDateTime;
use uuid::Uuid;

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn service_instance_inventory_is_stable_by_host_and_cursor() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let worker = worker_pool().await;
    let store = PostgresGatewayServiceTargets::new(worker);
    let host = format!("inventory-host-{}", Uuid::new_v4());
    let mut expected = HashSet::new();
    let mut expired_instance = None;
    for index in 0..130 {
        let fixture = seed_gateway_with_instance_state(
            &pool,
            &format!("inventory-{index}-{}", Uuid::new_v4()),
            "enabled",
            "published",
            "ready",
            index == 0,
            Some(&host),
        )
        .await;
        if index == 0 {
            expired_instance = Some(fixture.old_instance);
        }
        expected.insert(fixture.old_instance);
    }
    let foreign_host = format!("foreign-host-{}", Uuid::new_v4());
    let foreign = seed_gateway_with_instance_state(
        &pool,
        &format!("foreign-{}", Uuid::new_v4()),
        "enabled",
        "published",
        "ready",
        false,
        Some(&foreign_host),
    )
    .await;
    let cleaned = seed_gateway_with_instance_state(
        &pool,
        &format!("cleaned-{}", Uuid::new_v4()),
        "enabled",
        "published",
        "cleaned",
        false,
        Some(&host),
    )
    .await;

    let observed = tokio::time::timeout(Duration::from_secs(30), async {
        let mut cursor = None;
        let mut pages = 0;
        let mut observed = Vec::new();
        loop {
            pages += 1;
            assert!(pages <= 16, "inventory pagination exceeded bounded pages");
            let page = GatewayServiceInstancePage::new(host.clone(), cursor, 17)
                .expect("valid inventory page");
            let result = store
                .list_service_instances(page)
                .await
                .expect("list service instances");
            assert!(result.instances.len() <= 17);
            observed.extend(result.instances);
            let Some(next) = result.next_after else {
                break;
            };
            assert_ne!(Some(next), cursor);
            cursor = Some(next);
        }
        observed
    })
    .await
    .expect("inventory pagination completes");
    assert!(
        observed
            .windows(2)
            .all(|pair| pair[0].identity.instance_id < pair[1].identity.instance_id)
    );
    let observed_ids: HashSet<_> = observed
        .iter()
        .map(|lease| lease.identity.instance_id)
        .collect();
    assert_eq!(observed.len(), expected.len());
    assert_eq!(observed_ids, expected);
    let expired_instance = expired_instance.expect("expired inventory row");
    assert!(observed.iter().any(|lease| {
        lease.identity.instance_id == expired_instance
            && lease.lease_expires_at < OffsetDateTime::now_utc()
    }));
    assert!(
        observed
            .iter()
            .any(|lease| lease.lease_expires_at > OffsetDateTime::now_utc())
    );
    let owners: HashSet<_> = observed.iter().map(|lease| lease.owner_uuid).collect();
    assert!(owners.len() > 1);
    assert!(!observed_ids.contains(&foreign.old_instance));
    assert!(!observed_ids.contains(&cleaned.old_instance));
}
