//! maintenance page scenario.

use super::support::{seed_gateway_with_instance_state, test_pool, worker_pool};
use gateway_domain::{
    GatewayServiceLogMaintenanceProjectPage, GatewayServiceLogMaintenanceProjects,
    MAX_SERVICE_LOG_MAINTENANCE_PROJECT_PAGE_SIZE,
};
use gateway_postgres::PostgresGatewayServiceLogStore;
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use std::{env, time::Duration};
use uuid::Uuid;

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn service_log_maintenance_projects_page_is_bounded_and_worker_only() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let worker = worker_pool().await;
    let store = PostgresGatewayServiceLogStore::new(worker);
    let inactive = seed_gateway_with_instance_state(
        &pool,
        &format!("maintenance-inactive-{}", Uuid::new_v4()),
        "enabled",
        "published",
        "cleaned",
        true,
        None,
    )
    .await;
    sqlx::query(
        "INSERT INTO gateway_service_log_epochs
            (instance_id, gateway_id, revision_id, project_id, fencing_token,
             acknowledged_through)
         VALUES ($1, $2, $3, $4, 1, 4)",
    )
    .bind(inactive.old_instance)
    .bind(inactive.gateway)
    .bind(inactive.old_service)
    .bind(inactive.project)
    .execute(&pool)
    .await
    .expect("inactive retained epoch");
    sqlx::query(
        "INSERT INTO gateway_service_log_project_usage
            (project_id, retained_epochs, storage_dropped_chunks,
             storage_dropped_bytes, pressure_cleanup_pending)
         VALUES ($1, 1, 3, 41, true)",
    )
    .bind(inactive.project)
    .execute(&pool)
    .await
    .expect("inactive retained usage");
    let non_cleaned_instances: i64 = sqlx::query_scalar(
        "SELECT count(*)
           FROM gateway_service_instances AS instance
           JOIN gateways AS gateway ON gateway.id = instance.gateway_id
          WHERE gateway.project_id = $1 AND instance.state <> 'cleaned'",
    )
    .bind(inactive.project)
    .fetch_one(&pool)
    .await
    .expect("inactive instance state");
    assert_eq!(non_cleaned_instances, 0);

    let previous_max: Option<Uuid> = sqlx::query_scalar(
        "SELECT project_id FROM gateway_service_log_project_usage
          ORDER BY project_id DESC LIMIT 1",
    )
    .fetch_optional(&pool)
    .await
    .expect("existing maintenance project maximum");
    let base = previous_max.map_or(1, |value| {
        value
            .as_u128()
            .checked_add(1)
            .expect("room for maintenance project boundary")
    });
    base.checked_add(u128::from(MAX_SERVICE_LOG_MAINTENANCE_PROJECT_PAGE_SIZE))
        .expect("room for all maintenance project boundary IDs");
    let organization = Uuid::new_v4();
    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, $2)")
        .bind(organization)
        .bind(format!("maintenance-projects-{organization}"))
        .execute(&pool)
        .await
        .expect("maintenance project organization");

    let project_ids = (0..=MAX_SERVICE_LOG_MAINTENANCE_PROJECT_PAGE_SIZE)
        .map(|offset| Uuid::from_u128(base + u128::from(offset)))
        .collect::<Vec<_>>();
    let mut transaction = pool.begin().await.expect("maintenance project transaction");
    for (index, project_id) in project_ids.iter().copied().enumerate() {
        sqlx::query("INSERT INTO projects (id, organization_id, name) VALUES ($1, $2, $3)")
            .bind(project_id)
            .bind(organization)
            .bind(format!("maintenance-project-{index}-{project_id}"))
            .execute(&mut *transaction)
            .await
            .expect("maintenance project");
        sqlx::query(
            "INSERT INTO gateway_service_log_project_usage
                (project_id, retained_epochs, storage_dropped_chunks,
                 storage_dropped_bytes, pressure_cleanup_pending)
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(project_id)
        .bind(0_i32)
        .bind(0_i64)
        .bind(0_i64)
        .bind(false)
        .execute(&mut *transaction)
        .await
        .expect("maintenance project usage");
    }
    transaction
        .commit()
        .await
        .expect("maintenance project fixture commit");

    let inactive_seen = tokio::time::timeout(Duration::from_secs(30), async {
        let mut after = None;
        let mut seen = false;
        loop {
            let page = store
                .list_projects(
                    GatewayServiceLogMaintenanceProjectPage::new(
                        after,
                        MAX_SERVICE_LOG_MAINTENANCE_PROJECT_PAGE_SIZE,
                    )
                    .expect("maintenance project sweep page"),
                )
                .await
                .expect("maintenance project sweep");
            seen |= page.projects.contains(&inactive.project);
            let Some(next_after) = page.next_after else {
                break seen;
            };
            assert!(after.is_none_or(|previous| next_after > previous));
            after = Some(next_after);
        }
    })
    .await
    .expect("maintenance project sweep completes");
    assert!(
        inactive_seen,
        "inactive retained project must be enumerable"
    );

    let first = store
        .list_projects(
            GatewayServiceLogMaintenanceProjectPage::new(
                previous_max,
                MAX_SERVICE_LOG_MAINTENANCE_PROJECT_PAGE_SIZE,
            )
            .expect("first maintenance project page"),
        )
        .await
        .expect("first maintenance project page query");
    assert_eq!(first.projects.len(), 128);
    assert_eq!(first.projects.first(), Some(&project_ids[0]));
    assert_eq!(first.projects.last(), Some(&project_ids[127]));
    assert_eq!(first.next_after, Some(project_ids[127]));

    let second = store
        .list_projects(
            GatewayServiceLogMaintenanceProjectPage::new(first.next_after, 128)
                .expect("second maintenance project page"),
        )
        .await
        .expect("second maintenance project page query");
    assert_eq!(second.projects, vec![project_ids[128]]);
    assert_eq!(second.next_after, None);

    let mut listed = first.projects;
    listed.extend(second.projects);
    assert_eq!(listed, project_ids);

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
    let visible_project_ids: Vec<Uuid> =
        sqlx::query_scalar("SELECT project_id FROM gateway_service_log_project_usage LIMIT 1")
            .fetch_all(&app_pool)
            .await
            .expect("application role project-id query");
    assert!(
        visible_project_ids.is_empty(),
        "an actor-less application session must see no project usage rows"
    );
    let denied =
        sqlx::query("SELECT retained_bytes FROM gateway_service_log_project_usage LIMIT 1")
            .fetch_one(&app_pool)
            .await
            .expect_err("application role must not read retained usage");
    assert_eq!(
        denied
            .as_database_error()
            .and_then(sqlx::error::DatabaseError::code)
            .as_deref(),
        Some("42501")
    );
    app_pool.close().await;
}
