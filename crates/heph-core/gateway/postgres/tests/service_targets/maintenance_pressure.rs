//! maintenance pressure scenario.

use super::support::{
    batch_with_payload, seed_gateway_with_instance_state, test_pool, worker_pool,
};
use gateway_domain::{
    GatewayServiceIdentity, GatewayServiceLogAppendBatch, GatewayServiceLogMaintenance,
    GatewayServiceLogMaintenancePolicy, GatewayServiceLogMaintenanceReport, GatewayServiceLogStore,
    GatewayServiceOwner, GatewayServiceTargetStore, ServiceLogLoss, ServiceLogRecord,
};
use gateway_postgres::{PostgresGatewayServiceLogStore, PostgresGatewayServiceTargets};
use serial_test::serial;
use time::OffsetDateTime;
use uuid::Uuid;
use vm_trait::LogStream;

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn service_log_maintenance_combines_ttl_and_pressure_without_double_counting() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let worker = worker_pool().await;
    let fixture = seed_gateway_with_instance_state(
        &pool,
        &format!("mixed-{}", Uuid::new_v4()),
        "enabled",
        "published",
        "ready",
        false,
        None,
    )
    .await;
    let targets = PostgresGatewayServiceTargets::new(worker.clone());
    let identity = GatewayServiceIdentity {
        instance_id: fixture.old_instance,
        gateway_id: fixture.gateway,
        revision_id: fixture.old_service,
    };
    let lease = targets
        .get_service_instance(identity)
        .await
        .expect("mixed maintenance instance lookup")
        .expect("mixed maintenance instance");
    let owner = GatewayServiceOwner::new(lease.owner_host_id.clone(), lease.owner_uuid)
        .expect("mixed maintenance owner");
    let store = PostgresGatewayServiceLogStore::new(worker);
    store
        .append_batch(&lease, &owner, batch_with_payload(0, 64 * 1024))
        .await
        .expect("expired mixed payload");
    sqlx::query(
        "UPDATE gateway_service_log_chunks
            SET stored_at = clock_timestamp() - interval '25 hours'
          WHERE instance_id = $1 AND fencing_token = $2 AND sequence = 0",
    )
    .bind(fixture.old_instance)
    .bind(lease.fencing_token)
    .execute(&pool)
    .await
    .expect("age mixed payload");
    let fresh = GatewayServiceLogAppendBatch::new(
        (1..=56)
            .map(|sequence| ServiceLogRecord {
                sequence,
                stream: LogStream::Stdout,
                observed_at: OffsetDateTime::now_utc(),
                bytes: vec![b'm'; 64 * 1024],
            })
            .collect(),
        ServiceLogLoss::default(),
    )
    .expect("fresh mixed payloads");
    store
        .append_batch(&lease, &owner, fresh)
        .await
        .expect("fresh mixed append");
    let report = store
        .maintain_project(
            fixture.project,
            GatewayServiceLogMaintenancePolicy::new(256, 8),
        )
        .await
        .expect("mixed TTL and pressure pass");
    assert_eq!(
        report,
        GatewayServiceLogMaintenanceReport {
            expired_chunks: 1,
            expired_bytes: 64 * 1024,
            evicted_chunks: 8,
            evicted_bytes: 8 * 64 * 1024,
            ..GatewayServiceLogMaintenanceReport::default()
        }
    );
    let (actual_chunks, actual_bytes): (i64, i64) = sqlx::query_as(
        "SELECT count(*), coalesce(sum(octet_length(bytes)), 0)
           FROM gateway_service_log_chunks WHERE project_id = $1",
    )
    .bind(fixture.project)
    .fetch_one(&pool)
    .await
    .expect("mixed payload totals");
    let (usage_chunks, usage_bytes): (i64, i64) = sqlx::query_as(
        "SELECT retained_chunks, retained_bytes
           FROM gateway_service_log_project_usage WHERE project_id = $1",
    )
    .bind(fixture.project)
    .fetch_one(&pool)
    .await
    .expect("mixed durable totals");
    assert_eq!((actual_chunks, actual_bytes), (48, 48 * 64 * 1024));
    assert_eq!((usage_chunks, usage_bytes), (actual_chunks, actual_bytes));
}
