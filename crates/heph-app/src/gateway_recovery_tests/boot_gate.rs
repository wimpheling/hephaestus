use super::*;

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn boot_gate_holds_startup_for_unexpired_owned_inventory() {
    let Some(database) = isolated_startup_database().await else {
        return;
    };
    let pool = database.control.clone();
    let inventory = seed_fixture(&pool, "http.service.v1").await;
    let candidate = seed_fixture(&pool, "http.service.v1").await;
    let host_id = format!("recovery-test-{}", candidate.gateway.simple());
    let inventory_instance = inventory
        .service_instance
        .expect("inventory fixture instance");
    sqlx::query("DELETE FROM gateway_service_instances WHERE id = $1")
        .bind(inventory_instance)
        .execute(&pool)
        .await
        .expect("replace inventory claim");
    sqlx::query(
        "INSERT INTO gateway_service_instances
            (id, gateway_id, revision_id, owner_host_id, owner_uuid,
             fencing_token, vm_id, state, lease_expires_at, heartbeat_at)
         VALUES ($1, $2, $3, $6, $4, 1, $5, 'ready',
                 now() + interval '10 minutes', now())",
    )
    .bind(inventory_instance)
    .bind(inventory.gateway)
    .bind(inventory.revision)
    .bind(inventory.owner)
    .bind(format!("gateway-service-{inventory_instance}"))
    .bind(&host_id)
    .execute(&pool)
    .await
    .expect("assign live inventory to this daemon host");
    let (task, cancellation, caddy_started, caddy_release, destroyed, provisioned) =
        spawn_automatic_start_with_worker(
            &pool,
            &database.worker,
            candidate,
            true,
            false,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await;
    tokio::time::timeout(StdDuration::from_secs(10), caddy_started.notified())
        .await
        .expect("Caddy reconciliation starts while boot recovery waits");
    tokio::time::sleep(StdDuration::from_secs(2)).await;
    assert_eq!(destroyed.load(Ordering::Acquire), 0);
    assert_eq!(provisioned.load(Ordering::Acquire), 0);
    let provisioned: i64 = sqlx::query_scalar(
        "SELECT count(*)
           FROM gateway_service_instances
          WHERE gateway_id = $1 AND revision_id = $2",
    )
    .bind(candidate.gateway)
    .bind(candidate.revision)
    .fetch_one(&pool)
    .await
    .expect("read candidate claim count");
    assert_eq!(
        provisioned, 0,
        "boot inventory blocks candidate provisioning"
    );
    let inventory_count: i64 = sqlx::query_scalar(
        "SELECT count(*)
           FROM gateway_service_instances
          WHERE gateway_id = $1 AND revision_id = $2 AND state = 'ready'
            AND owner_host_id = $3",
    )
    .bind(inventory.gateway)
    .bind(inventory.revision)
    .bind(&host_id)
    .fetch_one(&pool)
    .await
    .expect("read retained inventory");
    assert_eq!(inventory_count, 1, "live host inventory remains present");
    cancellation.cancel();
    tokio::time::timeout(StdDuration::from_secs(10), task)
        .await
        .expect("boot-gated loop joins")
        .expect("boot-gated loop task");
    caddy_release.notify_one();
    cleanup_startup_fixture(&pool, candidate).await;
    cleanup_startup_fixture(&pool, inventory).await;
    drop_isolated_startup_database(database).await;
}
