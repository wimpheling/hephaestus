//! maintenance watermark scenario.

use super::support::{
    batch_with_sequence, seed_gateway_with_instance_state, test_pool, worker_pool,
};
use gateway_domain::{
    GatewayServiceIdentity, GatewayServiceLogMaintenance, GatewayServiceLogMaintenancePolicy,
    GatewayServiceLogStore, GatewayServiceOwner, GatewayServiceTargetStore,
};
use gateway_postgres::{PostgresGatewayServiceLogStore, PostgresGatewayServiceTargets};
use serial_test::serial;
use uuid::Uuid;

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn service_log_maintenance_preserves_watermark_and_gcs_empty_cleaned_epoch() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let worker = worker_pool().await;
    let fixture = seed_gateway_with_instance_state(
        &pool,
        &format!("gc-{}", Uuid::new_v4()),
        "enabled",
        "published",
        "ready",
        false,
        None,
    )
    .await;
    let other = seed_gateway_with_instance_state(
        &pool,
        &format!("other-{}", Uuid::new_v4()),
        "enabled",
        "published",
        "ready",
        false,
        None,
    )
    .await;
    let targets = PostgresGatewayServiceTargets::new(worker.clone());
    let lease = targets
        .get_service_instance(GatewayServiceIdentity {
            instance_id: fixture.old_instance,
            gateway_id: fixture.gateway,
            revision_id: fixture.old_service,
        })
        .await
        .expect("GC instance lookup")
        .expect("GC instance");
    let other_lease = targets
        .get_service_instance(GatewayServiceIdentity {
            instance_id: other.old_instance,
            gateway_id: other.gateway,
            revision_id: other.old_service,
        })
        .await
        .expect("other instance lookup")
        .expect("other instance");
    let owner =
        GatewayServiceOwner::new(lease.owner_host_id.clone(), lease.owner_uuid).expect("GC owner");
    let other_owner =
        GatewayServiceOwner::new(other_lease.owner_host_id.clone(), other_lease.owner_uuid)
            .expect("other owner");
    let store = PostgresGatewayServiceLogStore::new(worker);
    store
        .append_batch(&lease, &owner, batch_with_sequence(0))
        .await
        .expect("watermark payload");
    store
        .append_batch(&other_lease, &other_owner, batch_with_sequence(0))
        .await
        .expect("other project payload");
    sqlx::query(
        "UPDATE gateway_service_log_chunks
            SET stored_at = clock_timestamp() - interval '25 hours'
          WHERE instance_id = $1 AND fencing_token = $2",
    )
    .bind(fixture.old_instance)
    .bind(lease.fencing_token)
    .execute(&pool)
    .await
    .expect("age watermark payload");
    let expired = store
        .maintain_project(
            fixture.project,
            GatewayServiceLogMaintenancePolicy::new(8, 8),
        )
        .await
        .expect("watermark TTL pass");
    assert_eq!(expired.expired_chunks, 1);
    let replay = store
        .append_batch(&lease, &owner, batch_with_sequence(0))
        .await
        .expect("watermark replay");
    assert_eq!((replay.accepted_chunks, replay.duplicate_chunks), (0, 0));
    let (acknowledged, rows): (i64, i64) = sqlx::query_as(
        "SELECT acknowledged_through,
                (SELECT count(*) FROM gateway_service_log_chunks
                  WHERE instance_id = $1 AND fencing_token = $2)
           FROM gateway_service_log_epochs
          WHERE instance_id = $1 AND fencing_token = $2",
    )
    .bind(fixture.old_instance)
    .bind(lease.fencing_token)
    .fetch_one(&pool)
    .await
    .expect("watermark state");
    assert_eq!((acknowledged, rows), (0, 0));

    let cleaned_instance = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO gateway_service_instances
            (id, gateway_id, revision_id, owner_host_id, owner_uuid,
             fencing_token, vm_id, state, lease_expires_at, heartbeat_at, cleaned_at)
         VALUES ($1, $2, $3, $4, $5, 1, $6, 'cleaned',
                 clock_timestamp() - interval '1 day',
                 clock_timestamp() - interval '2 days', clock_timestamp() - interval '2 days')",
    )
    .bind(cleaned_instance)
    .bind(fixture.gateway)
    .bind(fixture.old_service)
    .bind(format!("gc-cleaned-{}", Uuid::new_v4()))
    .bind(Uuid::new_v4())
    .bind(format!("gateway-service-{cleaned_instance}"))
    .execute(&pool)
    .await
    .expect("cleaned GC instance");
    sqlx::query(
        "INSERT INTO gateway_service_log_epochs
            (instance_id, gateway_id, revision_id, project_id, fencing_token,
             updated_at)
         VALUES ($1, $2, $3, $4, 1, clock_timestamp() - interval '25 hours')",
    )
    .bind(cleaned_instance)
    .bind(fixture.gateway)
    .bind(fixture.old_service)
    .bind(fixture.project)
    .execute(&pool)
    .await
    .expect("empty GC epoch");
    sqlx::query(
        "UPDATE gateway_service_log_project_usage
            SET retained_epochs = retained_epochs + 1
          WHERE project_id = $1",
    )
    .bind(fixture.project)
    .execute(&pool)
    .await
    .expect("GC epoch usage");
    let metadata = store
        .maintain_project(
            fixture.project,
            GatewayServiceLogMaintenancePolicy::new(8, 8),
        )
        .await
        .expect("metadata GC pass");
    assert_eq!(metadata.metadata_epochs, 1);
    let retained_epochs: i32 = sqlx::query_scalar(
        "SELECT retained_epochs FROM gateway_service_log_project_usage WHERE project_id = $1",
    )
    .bind(fixture.project)
    .fetch_one(&pool)
    .await
    .expect("GC retained epoch count");
    assert_eq!(retained_epochs, 1);
    let cleaned_rows: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM gateway_service_log_epochs WHERE instance_id = $1",
    )
    .bind(cleaned_instance)
    .fetch_one(&pool)
    .await
    .expect("GC epoch absence");
    assert_eq!(cleaned_rows, 0);
    let other_rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM gateway_service_log_chunks WHERE project_id = $1")
            .bind(other.project)
            .fetch_one(&pool)
            .await
            .expect("other project remains");
    assert_eq!(other_rows, 1);
}
