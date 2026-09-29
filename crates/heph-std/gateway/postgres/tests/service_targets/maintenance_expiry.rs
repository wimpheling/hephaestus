//! maintenance expiry scenario.

use super::support::{
    batch_with_payloads, batch_with_sequence, seed_gateway_with_instance_state, test_pool,
    worker_pool,
};
use gateway_domain::{
    GatewayServiceIdentity, GatewayServiceLogMaintenance, GatewayServiceLogMaintenancePolicy,
    GatewayServiceLogStore, GatewayServiceOwner, GatewayServiceTargetStore,
};
use gateway_postgres::{PostgresGatewayServiceLogStore, PostgresGatewayServiceTargets};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use std::env;
use uuid::Uuid;

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn service_log_maintenance_expires_evicts_and_denies_app_role() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let worker = worker_pool().await;
    let fixture = seed_gateway_with_instance_state(
        &pool,
        &format!("maint-{}", Uuid::new_v4()),
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
        .expect("maintenance instance lookup")
        .expect("maintenance instance");
    let owner = GatewayServiceOwner::new(lease.owner_host_id.clone(), lease.owner_uuid)
        .expect("maintenance owner");
    let store = PostgresGatewayServiceLogStore::new(worker);
    store
        .append_batch(&lease, &owner, batch_with_sequence(0))
        .await
        .expect("expired maintenance payload");
    sqlx::query(
        "UPDATE gateway_service_log_chunks
            SET stored_at = clock_timestamp() - interval '25 hours'
          WHERE instance_id = $1 AND fencing_token = $2 AND sequence = 0",
    )
    .bind(fixture.old_instance)
    .bind(lease.fencing_token)
    .execute(&pool)
    .await
    .expect("age maintenance payload");
    let expired = store
        .maintain_project(
            fixture.project,
            GatewayServiceLogMaintenancePolicy::new(8, 8),
        )
        .await
        .expect("maintenance TTL pass");
    assert_eq!(expired.expired_chunks, 1);
    assert_eq!(expired.expired_bytes, 18);
    store
        .append_batch(&lease, &owner, batch_with_payloads(1, 56, 64 * 1024))
        .await
        .expect("pressure maintenance payloads");
    for pass in 0..8 {
        let report = store
            .maintain_project(
                fixture.project,
                GatewayServiceLogMaintenancePolicy::new(1, 8),
            )
            .await
            .expect("pressure continuation pass");
        assert_eq!(report.evicted_chunks, 1, "pressure pass {pass}");
        assert_eq!(report.evicted_bytes, 64 * 1024, "pressure pass {pass}");
        let (rows, bytes): (i64, i64) = sqlx::query_as(
            "SELECT count(*), coalesce(sum(octet_length(bytes)), 0)
               FROM gateway_service_log_chunks WHERE project_id = $1",
        )
        .bind(fixture.project)
        .fetch_one(&pool)
        .await
        .expect("payload totals");
        let usage: (i64, i64) = sqlx::query_as(
            "SELECT retained_chunks, retained_bytes
               FROM gateway_service_log_project_usage WHERE project_id = $1",
        )
        .bind(fixture.project)
        .fetch_one(&pool)
        .await
        .expect("usage totals");
        assert_eq!(usage, (rows, bytes));
        assert_eq!(report.has_more, pass < 7, "continuation pass {pass}");
    }
    let pending: bool = sqlx::query_scalar(
        "SELECT pressure_cleanup_pending FROM gateway_service_log_epochs
          WHERE instance_id = $1 AND fencing_token = $2",
    )
    .bind(fixture.old_instance)
    .bind(lease.fencing_token)
    .fetch_one(&pool)
    .await
    .expect("pressure flag");
    assert!(!pending);
    let database_url = env::var("HEPHAESTUS_POSTGRES_TEST_URL").expect("test database URL");
    let app_pool = PgPoolOptions::new()
        .max_connections(1)
        .after_connect(|connection, _metadata| {
            Box::pin(async move {
                sqlx::query("SET ROLE hephaestus_app")
                    .execute(&mut *connection)
                    .await?;
                Ok(())
            })
        })
        .connect(&database_url)
        .await
        .expect("application role pool");
    let denied = sqlx::query("DELETE FROM gateway_service_log_chunks WHERE project_id = $1")
        .bind(fixture.project)
        .execute(&app_pool)
        .await;
    assert!(
        denied.is_err(),
        "application role cannot run maintenance DELETE"
    );
}
