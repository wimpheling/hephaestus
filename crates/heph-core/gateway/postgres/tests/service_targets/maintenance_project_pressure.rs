//! maintenance project pressure scenario.

use super::support::{seed_gateway_with_instance_state, test_pool, worker_pool};
use gateway_domain::{GatewayServiceLogMaintenance, GatewayServiceLogMaintenancePolicy};
use gateway_postgres::PostgresGatewayServiceLogStore;
use serial_test::serial;
use uuid::Uuid;

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn service_log_project_pressure_stops_before_unpressured_instance() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let worker = worker_pool().await;
    let fixture = seed_gateway_with_instance_state(
        &pool,
        &format!("project-{}", Uuid::new_v4()),
        "enabled",
        "published",
        "ready",
        false,
        None,
    )
    .await;
    let chunk_bytes = vec![b'p'; 64 * 1024];
    let newest_instance = Uuid::new_v4();
    for index in 0..19 {
        let instance_id = if index == 18 {
            newest_instance
        } else {
            Uuid::new_v4()
        };
        let owner_uuid = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO gateway_service_instances
                (id, gateway_id, revision_id, owner_host_id, owner_uuid,
                 fencing_token, vm_id, state, lease_expires_at, heartbeat_at, cleaned_at)
             VALUES ($1, $2, $3, $4, $5, 1, $6, 'cleaned',
                     clock_timestamp() - interval '1 minute',
                     clock_timestamp() - interval '2 minutes', clock_timestamp())",
        )
        .bind(instance_id)
        .bind(fixture.gateway)
        .bind(fixture.old_service)
        .bind(format!("project-pressure-{index}-{}", Uuid::new_v4()))
        .bind(owner_uuid)
        .bind(format!("gateway-service-{instance_id}"))
        .execute(&pool)
        .await
        .expect("project pressure instance");
        let count = 49_i64;
        let older = index < 18;
        sqlx::query(
            "INSERT INTO gateway_service_log_epochs
                (instance_id, gateway_id, revision_id, project_id, fencing_token,
                 retained_bytes, retained_chunks)
             VALUES ($1, $2, $3, $4, 1, $5, $6)",
        )
        .bind(instance_id)
        .bind(fixture.gateway)
        .bind(fixture.old_service)
        .bind(fixture.project)
        .bind(count * 64 * 1024)
        .bind(count)
        .execute(&pool)
        .await
        .expect("pressure epoch");
        for sequence in 0..count {
            sqlx::query(
                "INSERT INTO gateway_service_log_chunks
                    (instance_id, gateway_id, revision_id, project_id,
                     fencing_token, sequence, stream, observed_at, bytes, stored_at)
                 VALUES ($1, $2, $3, $4, 1, $5, 'stdout', clock_timestamp(), $6,
                         CASE WHEN $7 THEN clock_timestamp() - interval '1 hour'
                              ELSE clock_timestamp() END)",
            )
            .bind(instance_id)
            .bind(fixture.gateway)
            .bind(fixture.old_service)
            .bind(fixture.project)
            .bind(sequence)
            .bind(&chunk_bytes)
            .bind(older)
            .execute(&pool)
            .await
            .expect("pressure payload");
        }
    }
    let total_chunks = 19 * 49;
    sqlx::query(
        "INSERT INTO gateway_service_log_project_usage
            (project_id, retained_bytes, retained_chunks, retained_epochs)
         VALUES ($1, $2, $3, 19)",
    )
    .bind(fixture.project)
    .bind(total_chunks * 64 * 1024)
    .bind(total_chunks)
    .execute(&pool)
    .await
    .expect("project pressure usage");
    let store = PostgresGatewayServiceLogStore::new(worker);
    let report = store
        .maintain_project(
            fixture.project,
            GatewayServiceLogMaintenancePolicy::new(256, 8),
        )
        .await
        .expect("project pressure pass");
    assert_eq!(report.evicted_chunks, 163);
    assert_eq!(report.evicted_bytes, 163 * 64 * 1024);
    let remaining_newest: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM gateway_service_log_chunks WHERE instance_id = $1",
    )
    .bind(newest_instance)
    .fetch_one(&pool)
    .await
    .expect("project pressure remaining rows");
    assert_eq!(
        remaining_newest, 49,
        "unpressured instance must remain intact"
    );
    let (usage_chunks, usage_bytes): (i64, i64) = sqlx::query_as(
        "SELECT retained_chunks, retained_bytes
           FROM gateway_service_log_project_usage WHERE project_id = $1",
    )
    .bind(fixture.project)
    .fetch_one(&pool)
    .await
    .expect("project pressure usage after pass");
    assert_eq!((usage_chunks, usage_bytes), (768, 768 * 64 * 1024));
}
