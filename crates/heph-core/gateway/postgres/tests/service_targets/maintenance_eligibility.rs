//! maintenance eligibility scenario.

use super::support::{seed_gateway_with_instance_state, test_pool, worker_pool};
use gateway_domain::{
    GatewayServiceLogMaintenance, GatewayServiceLogMaintenancePolicy, GatewayServiceOwner,
    GatewayServiceOwnership,
};
use gateway_postgres::{PostgresGatewayServiceLogStore, PostgresGatewayServiceOwnership};
use serial_test::serial;
use std::time::Duration;
use uuid::Uuid;

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn service_log_maintenance_gc_eligibility_matrix() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let worker = worker_pool().await;
    let fixture = seed_gateway_with_instance_state(
        &pool,
        &format!("matrix-{}", Uuid::new_v4()),
        "enabled",
        "published",
        "ready",
        true,
        None,
    )
    .await;
    sqlx::query(
        "INSERT INTO gateway_service_log_epochs
            (instance_id, gateway_id, revision_id, project_id, fencing_token, updated_at)
         VALUES ($1, $2, $3, $4, 1, clock_timestamp() - interval '25 hours')",
    )
    .bind(fixture.old_instance)
    .bind(fixture.gateway)
    .bind(fixture.old_service)
    .bind(fixture.project)
    .execute(&pool)
    .await
    .expect("expired same-fence epoch");
    let mut old_cleaned = Vec::new();
    for index in 0..2 {
        let instance_id = Uuid::new_v4();
        old_cleaned.push(instance_id);
        sqlx::query(
            "INSERT INTO gateway_service_instances
                (id, gateway_id, revision_id, owner_host_id, owner_uuid,
                 fencing_token, vm_id, state, lease_expires_at, heartbeat_at, cleaned_at)
             VALUES ($1, $2, $3, $4, $5, 1, $6, 'cleaned',
                     clock_timestamp() - interval '1 day',
                     clock_timestamp() - interval '2 days', clock_timestamp())",
        )
        .bind(instance_id)
        .bind(fixture.gateway)
        .bind(fixture.old_service)
        .bind(format!("matrix-old-{index}-{}", Uuid::new_v4()))
        .bind(Uuid::new_v4())
        .bind(format!("gateway-service-{instance_id}"))
        .execute(&pool)
        .await
        .expect("old cleaned instance");
        sqlx::query(
            "INSERT INTO gateway_service_log_epochs
                (instance_id, gateway_id, revision_id, project_id, fencing_token, updated_at)
             VALUES ($1, $2, $3, $4, 1, clock_timestamp() - interval '25 hours')",
        )
        .bind(instance_id)
        .bind(fixture.gateway)
        .bind(fixture.old_service)
        .bind(fixture.project)
        .execute(&pool)
        .await
        .expect("old cleaned epoch");
    }
    let recent_instance = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO gateway_service_instances
            (id, gateway_id, revision_id, owner_host_id, owner_uuid,
             fencing_token, vm_id, state, lease_expires_at, heartbeat_at, cleaned_at)
         VALUES ($1, $2, $3, $4, $5, 1, $6, 'cleaned',
                 clock_timestamp() - interval '1 day',
                 clock_timestamp() - interval '2 days', clock_timestamp())",
    )
    .bind(recent_instance)
    .bind(fixture.gateway)
    .bind(fixture.old_service)
    .bind(format!("matrix-recent-{}", Uuid::new_v4()))
    .bind(Uuid::new_v4())
    .bind(format!("gateway-service-{recent_instance}"))
    .execute(&pool)
    .await
    .expect("recent cleaned instance");
    sqlx::query(
        "INSERT INTO gateway_service_log_epochs
            (instance_id, gateway_id, revision_id, project_id, fencing_token)
         VALUES ($1, $2, $3, $4, 1)",
    )
    .bind(recent_instance)
    .bind(fixture.gateway)
    .bind(fixture.old_service)
    .bind(fixture.project)
    .execute(&pool)
    .await
    .expect("recent cleaned epoch");
    sqlx::query(
        "INSERT INTO gateway_service_log_project_usage
            (project_id, retained_epochs) VALUES ($1, 4)",
    )
    .bind(fixture.project)
    .execute(&pool)
    .await
    .expect("matrix epoch usage");
    let store = PostgresGatewayServiceLogStore::new(worker);
    for (pass, expected_more) in [(0, true), (1, false)] {
        let report = store
            .maintain_project(
                fixture.project,
                GatewayServiceLogMaintenancePolicy::new(8, 1),
            )
            .await
            .expect("matrix GC pass");
        assert_eq!(report.metadata_epochs, 1, "matrix pass {pass}");
        assert_eq!(report.has_more, expected_more, "matrix continuation {pass}");
        let retained_epochs: i32 = sqlx::query_scalar(
            "SELECT retained_epochs FROM gateway_service_log_project_usage
              WHERE project_id = $1",
        )
        .bind(fixture.project)
        .fetch_one(&pool)
        .await
        .expect("matrix retained epoch count");
        let actual_epochs: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM gateway_service_log_epochs WHERE project_id = $1",
        )
        .bind(fixture.project)
        .fetch_one(&pool)
        .await
        .expect("matrix actual epoch count");
        assert_eq!(i64::from(retained_epochs), actual_epochs);
    }
    let (live_rows, recent_rows): (i64, i64) = sqlx::query_as(
        "SELECT
            (SELECT count(*) FROM gateway_service_log_epochs WHERE instance_id = $1),
            (SELECT count(*) FROM gateway_service_log_epochs WHERE instance_id = $2)",
    )
    .bind(fixture.old_instance)
    .bind(recent_instance)
    .fetch_one(&pool)
    .await
    .expect("matrix protected rows");
    assert_eq!(live_rows, 1, "expired same-fence epoch remains");
    assert_eq!(recent_rows, 1, "recent cleaned epoch remains");
    for instance_id in old_cleaned {
        let rows: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM gateway_service_log_epochs WHERE instance_id = $1",
        )
        .bind(instance_id)
        .fetch_one(&pool)
        .await
        .expect("old cleaned row count");
        assert_eq!(rows, 0);
    }

    let advanced = seed_gateway_with_instance_state(
        &pool,
        &format!("advanced-{}", Uuid::new_v4()),
        "enabled",
        "published",
        "ready",
        true,
        None,
    )
    .await;
    let ownership = PostgresGatewayServiceOwnership::new(pool.clone());
    let advanced_owner = GatewayServiceOwner::new(advanced.owner_host_id.clone(), Uuid::new_v4())
        .expect("advanced owner");
    let advanced_lease = ownership
        .claim_expired(&advanced_owner, Duration::from_secs(60), 1)
        .await
        .expect("advance instance fence")[0]
        .clone();
    assert_eq!(advanced_lease.fencing_token, 2);
    sqlx::query(
        "INSERT INTO gateway_service_log_epochs
            (instance_id, gateway_id, revision_id, project_id, fencing_token, updated_at)
         VALUES ($1, $2, $3, $4, 1, clock_timestamp() - interval '25 hours')",
    )
    .bind(advanced.old_instance)
    .bind(advanced.gateway)
    .bind(advanced.old_service)
    .bind(advanced.project)
    .execute(&pool)
    .await
    .expect("advanced old-fence epoch");
    sqlx::query(
        "INSERT INTO gateway_service_log_project_usage
            (project_id, retained_epochs) VALUES ($1, 1)",
    )
    .bind(advanced.project)
    .execute(&pool)
    .await
    .expect("advanced epoch usage");
    let advanced_report = store
        .maintain_project(
            advanced.project,
            GatewayServiceLogMaintenancePolicy::new(8, 1),
        )
        .await
        .expect("advanced-fence GC pass");
    assert_eq!(advanced_report.metadata_epochs, 1);
    let advanced_rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM gateway_service_log_epochs WHERE project_id = $1")
            .bind(advanced.project)
            .fetch_one(&pool)
            .await
            .expect("advanced epoch removal");
    assert_eq!(advanced_rows, 0);
}
