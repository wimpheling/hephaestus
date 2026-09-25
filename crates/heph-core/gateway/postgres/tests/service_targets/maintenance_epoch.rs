//! maintenance epoch scenario.

use super::support::{
    batch_with_sequence, seed_gateway_with_instance_state, test_pool, worker_pool,
};
use gateway_domain::{
    GatewayServiceIdentity, GatewayServiceLogMaintenance, GatewayServiceLogMaintenancePolicy,
    GatewayServiceLogStore, GatewayServiceLogStoreError, GatewayServiceOwner,
    GatewayServiceTargetStore,
};
use gateway_postgres::{
    PostgresGatewayServiceLogReader, PostgresGatewayServiceLogStore, PostgresGatewayServiceTargets,
};
use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use std::{env, sync::Arc};
use uuid::Uuid;

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn service_log_epoch_cap_is_persisted_as_terminal_loss() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let worker = worker_pool().await;
    let targets = PostgresGatewayServiceTargets::new(worker.clone());
    let fixture = seed_gateway_with_instance_state(
        &pool,
        &format!("log-epoch-cap-{}", Uuid::new_v4()),
        "enabled",
        "published",
        "cleaned",
        true,
        None,
    )
    .await;
    sqlx::query(
        "INSERT INTO organization_members (organization_id, user_id, role)
         SELECT organization_id, $2, 'owner' FROM projects WHERE id = $1",
    )
    .bind(fixture.project)
    .bind(fixture.owner)
    .execute(&pool)
    .await
    .expect("cap fixture project owner");
    let live_instance = Uuid::new_v4();
    let live_owner = Uuid::new_v4();
    let live_host = format!("epoch-cap-live-{}", Uuid::new_v4());
    sqlx::query(
        "INSERT INTO gateway_service_instances
            (id, gateway_id, revision_id, owner_host_id, owner_uuid,
             fencing_token, vm_id, state, lease_expires_at, heartbeat_at)
         VALUES ($1, $2, $3, $4, $5, 128, $6, 'ready',
                 clock_timestamp() + interval '10 minutes', clock_timestamp())",
    )
    .bind(live_instance)
    .bind(fixture.gateway)
    .bind(fixture.old_service)
    .bind(&live_host)
    .bind(live_owner)
    .bind(format!("gateway-service-{live_instance}"))
    .execute(&pool)
    .await
    .expect("epoch-cap live instance");
    for fencing_token in 1_i64..=127 {
        sqlx::query(
            "INSERT INTO gateway_service_log_epochs
                (instance_id, gateway_id, revision_id, project_id, fencing_token,
                 updated_at)
             VALUES ($1, $2, $3, $4, $5, clock_timestamp())",
        )
        .bind(live_instance)
        .bind(fixture.gateway)
        .bind(fixture.old_service)
        .bind(fixture.project)
        .bind(fencing_token)
        .execute(&pool)
        .await
        .expect("protected historical epoch");
    }
    sqlx::query(
        "INSERT INTO gateway_service_log_epochs
            (instance_id, gateway_id, revision_id, project_id, fencing_token,
             updated_at)
         VALUES ($1, $2, $3, $4, 1,
                 clock_timestamp() - interval '25 hours')",
    )
    .bind(fixture.old_instance)
    .bind(fixture.gateway)
    .bind(fixture.old_service)
    .bind(fixture.project)
    .execute(&pool)
    .await
    .expect("old cleaned epoch");
    sqlx::query(
        "INSERT INTO gateway_service_log_project_usage
            (project_id, retained_epochs)
         SELECT $1, count(*)::integer
           FROM gateway_service_log_epochs
          WHERE project_id = $1",
    )
    .bind(fixture.project)
    .execute(&pool)
    .await
    .expect("persisted epoch count");
    let actual_epoch_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM gateway_service_log_epochs WHERE project_id = $1")
            .bind(fixture.project)
            .fetch_one(&pool)
            .await
            .expect("actual epoch count");
    assert_eq!(actual_epoch_count, 128);
    let identity = GatewayServiceIdentity {
        instance_id: live_instance,
        gateway_id: fixture.gateway,
        revision_id: fixture.old_service,
    };
    let targets_lease = targets
        .get_service_instance(identity)
        .await
        .expect("exact capped instance lookup")
        .expect("capped fixture instance");
    assert_eq!(targets_lease.fencing_token, 128);
    let owner = GatewayServiceOwner::new(
        targets_lease.owner_host_id.clone(),
        targets_lease.owner_uuid,
    )
    .expect("epoch-cap live owner");
    let store = PostgresGatewayServiceLogStore::new(worker);
    assert!(matches!(
        store
            .append_batch(&targets_lease, &owner, batch_with_sequence(0))
            .await,
        Err(GatewayServiceLogStoreError::Capacity)
    ));
    let app = PgPoolOptions::new()
        .max_connections(1)
        .after_connect(|connection, _metadata| {
            Box::pin(async move {
                sqlx::query("SET ROLE hephaestus_app")
                    .execute(&mut *connection)
                    .await?;
                sqlx::query("SET application_name = 'gateway-targets-cap-reader'")
                    .execute(&mut *connection)
                    .await?;
                Ok(())
            })
        })
        .connect(&env::var("HEPHAESTUS_POSTGRES_TEST_URL").expect("test database URL"))
        .await
        .expect("connect cap reader application role");
    let reader = PostgresGatewayServiceLogReader::new(
        app.clone(),
        Arc::new(authz_postgres::PostgresMelangeAuthorizer),
    );
    let reader_identity = AuthenticatedIdentity::new(
        UserId::from_uuid(fixture.owner),
        "target-cap-reader",
        "target cap reader",
        serde_json::json!({}),
        RequestId::new(),
    );
    let rejected_metadata = reader
        .get_project_metadata(&reader_identity, fixture.project)
        .await
        .expect("authorized reader sees persisted cap rejection");
    assert!(rejected_metadata.usage_present);
    assert_eq!(
        (
            rejected_metadata.storage_dropped_chunks,
            rejected_metadata.storage_dropped_bytes
        ),
        (1, 18)
    );
    let epoch_count: i32 = sqlx::query_scalar(
        "SELECT retained_epochs FROM gateway_service_log_project_usage WHERE project_id = $1",
    )
    .bind(fixture.project)
    .fetch_one(&pool)
    .await
    .expect("persisted epoch cap");
    assert_eq!(epoch_count, 128);
    let report = store
        .maintain_project(
            fixture.project,
            GatewayServiceLogMaintenancePolicy::new(1, 1),
        )
        .await
        .expect("reclaim old cleaned epoch");
    assert_eq!(report.metadata_epochs, 1);
    assert_eq!(report.expired_chunks, 0);
    assert_eq!(report.evicted_chunks, 0);
    let epoch_count: i32 = sqlx::query_scalar(
        "SELECT retained_epochs FROM gateway_service_log_project_usage WHERE project_id = $1",
    )
    .bind(fixture.project)
    .fetch_one(&pool)
    .await
    .expect("persisted post-GC epoch count");
    assert_eq!(epoch_count, 127);
    let actual_epoch_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM gateway_service_log_epochs WHERE project_id = $1")
            .bind(fixture.project)
            .fetch_one(&pool)
            .await
            .expect("actual post-GC epoch count");
    assert_eq!(actual_epoch_count, 127);
    let second = store
        .append_batch(&targets_lease, &owner, batch_with_sequence(1))
        .await
        .expect("new epoch after metadata reclamation");
    assert_eq!(second.accepted_chunks, 1);
    let epoch_count: i32 = sqlx::query_scalar(
        "SELECT retained_epochs FROM gateway_service_log_project_usage WHERE project_id = $1",
    )
    .bind(fixture.project)
    .fetch_one(&pool)
    .await
    .expect("persisted recreated epoch count");
    assert_eq!(epoch_count, 128);
    let (old_sequence, new_sequence): (i64, i64) = sqlx::query_as(
        "SELECT
            count(*) FILTER (WHERE sequence = 0),
            count(*) FILTER (WHERE sequence = 1)
           FROM gateway_service_log_chunks
          WHERE instance_id = $1 AND fencing_token = 128",
    )
    .bind(live_instance)
    .fetch_one(&pool)
    .await
    .expect("new epoch payload rows");
    assert_eq!(
        old_sequence, 0,
        "capacity-dropped batch was not resurrected"
    );
    assert_eq!(new_sequence, 1);
    let preserved_metadata = reader
        .get_project_metadata(&reader_identity, fixture.project)
        .await
        .expect("authorized reader sees cap rejection after GC");
    assert_eq!(
        (
            preserved_metadata.storage_dropped_chunks,
            preserved_metadata.storage_dropped_bytes
        ),
        (1, 18)
    );
    app.close().await;
}
