//! log append scenario.

use super::support::{
    batch_with_sequence, insert_disabled_service_revision, seed_gateway_with_instance_state,
    test_pool, worker_pool,
};
use gateway_domain::{
    GatewayServiceIdentity, GatewayServiceInstanceLease, GatewayServiceLogAppendBatch,
    GatewayServiceLogStore, GatewayServiceLogStoreError, GatewayServiceOwner,
    GatewayServiceTargetStore, ServiceLogLoss, ServiceLogRecord,
};
use gateway_postgres::{PostgresGatewayServiceLogStore, PostgresGatewayServiceTargets};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use std::env;
use time::OffsetDateTime;
use uuid::Uuid;
use vm_trait::LogStream;

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn service_log_append_is_worker_bound_and_idempotent() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let worker = worker_pool().await;
    let targets = PostgresGatewayServiceTargets::new(worker.clone());
    let fixture = seed_gateway_with_instance_state(
        &pool,
        &format!("log-{}", Uuid::new_v4()),
        "enabled",
        "published",
        "ready",
        false,
        None,
    )
    .await;
    let identity = GatewayServiceIdentity {
        instance_id: fixture.old_instance,
        gateway_id: fixture.gateway,
        revision_id: fixture.old_service,
    };
    let lease = targets
        .get_service_instance(identity)
        .await
        .expect("exact service instance lookup")
        .expect("seeded ready instance");
    let owner = GatewayServiceOwner::new(lease.owner_host_id.clone(), lease.owner_uuid)
        .expect("valid fixture owner");
    let store = PostgresGatewayServiceLogStore::new(worker);
    let batch = || batch_with_sequence(0);
    let accepted = store
        .append_batch(&lease, &owner, batch())
        .await
        .expect("worker append");
    assert_eq!(accepted.accepted_chunks, 1);
    assert_eq!(accepted.duplicate_chunks, 0);
    assert_eq!(accepted.retained_instance_chunks, 1);
    assert_eq!(accepted.retained_instance_bytes, 18);
    let retained_epochs: i32 = sqlx::query_scalar(
        "SELECT retained_epochs FROM gateway_service_log_project_usage WHERE project_id = $1",
    )
    .bind(fixture.project)
    .fetch_one(&pool)
    .await
    .expect("persisted first epoch count");
    assert_eq!(retained_epochs, 1);
    let duplicate = store
        .append_batch(&lease, &owner, batch())
        .await
        .expect("idempotent worker replay");
    assert_eq!(duplicate.accepted_chunks, 0);
    assert_eq!(duplicate.duplicate_chunks, 1);
    let retained_epochs_after_duplicate: i32 = sqlx::query_scalar(
        "SELECT retained_epochs FROM gateway_service_log_project_usage WHERE project_id = $1",
    )
    .bind(fixture.project)
    .fetch_one(&pool)
    .await
    .expect("persisted duplicate epoch count");
    assert_eq!(retained_epochs_after_duplicate, 1);
    let mismatch = GatewayServiceLogAppendBatch::new(
        vec![ServiceLogRecord {
            sequence: 0,
            stream: LogStream::Stdout,
            observed_at: OffsetDateTime::now_utc(),
            bytes: b"changed durable line".to_vec(),
        }],
        ServiceLogLoss::default(),
    )
    .expect("valid conflicting batch");
    assert!(matches!(
        store.append_batch(&lease, &owner, mismatch).await,
        Err(GatewayServiceLogStoreError::Conflict)
    ));

    // A second retained epoch contributes to the instance-wide quota and
    // result counters even though the current lease writes epoch one.
    sqlx::query(
        "INSERT INTO gateway_service_log_epochs
            (instance_id, gateway_id, revision_id, project_id, fencing_token,
             retained_bytes, retained_chunks)
         VALUES ($1, $2, $3, $4, 999, 1, 1)",
    )
    .bind(fixture.old_instance)
    .bind(fixture.gateway)
    .bind(fixture.old_service)
    .bind(fixture.project)
    .execute(&pool)
    .await
    .expect("second log epoch");
    sqlx::query(
        "UPDATE gateway_service_log_project_usage
            SET retained_bytes = retained_bytes + 1,
                retained_chunks = retained_chunks + 1,
                retained_epochs = retained_epochs + 1
          WHERE project_id = $1",
    )
    .bind(fixture.project)
    .execute(&pool)
    .await
    .expect("instance quota accounting");
    let second = store
        .append_batch(&lease, &owner, batch_with_sequence(1))
        .await
        .expect("append with multiple epochs");
    assert_eq!(second.retained_instance_chunks, 3);
    assert_eq!(second.retained_instance_bytes, 37);

    // Once durable capacity is full, the sequence is acknowledged as a
    // storage loss and cannot be resurrected by a retry.
    sqlx::query(
        "UPDATE gateway_service_log_epochs
            SET retained_bytes = 4194304, retained_chunks = 4096
          WHERE instance_id = $1 AND fencing_token = $2",
    )
    .bind(fixture.old_instance)
    .bind(lease.fencing_token)
    .execute(&pool)
    .await
    .expect("fill instance quota");
    let dropped = store
        .append_batch(&lease, &owner, batch_with_sequence(2))
        .await
        .expect("storage rejection is accounted");
    assert_eq!(dropped.storage_dropped_chunks, 1);
    let (dropped_chunks, dropped_bytes): (i64, i64) = sqlx::query_as(
        "SELECT storage_dropped_chunks, storage_dropped_bytes
           FROM gateway_service_log_epochs
          WHERE instance_id = $1 AND fencing_token = $2",
    )
    .bind(fixture.old_instance)
    .bind(lease.fencing_token)
    .fetch_one(&pool)
    .await
    .expect("durable storage loss");
    assert_eq!((dropped_chunks, dropped_bytes), (1, 18));
    sqlx::query(
        "DELETE FROM gateway_service_log_chunks
          WHERE instance_id = $1 AND fencing_token = $2 AND sequence = 0",
    )
    .bind(fixture.old_instance)
    .bind(lease.fencing_token)
    .execute(&pool)
    .await
    .expect("remove payload while retaining watermark");
    let replay = store
        .append_batch(&lease, &owner, batch())
        .await
        .expect("acknowledged replay is ignored");
    assert_eq!((replay.accepted_chunks, replay.duplicate_chunks), (0, 0));

    let (rows, bytes): (i64, i64) = sqlx::query_as(
        "SELECT count(*), coalesce(sum(octet_length(bytes)), 0)
           FROM gateway_service_log_chunks
          WHERE instance_id = $1 AND fencing_token = $2",
    )
    .bind(fixture.old_instance)
    .bind(lease.fencing_token)
    .fetch_one(&pool)
    .await
    .expect("durable log row");
    assert_eq!((rows, bytes), (1, 18));
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
    let app_visible: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM gateway_service_log_chunks
          WHERE instance_id = $1 AND fencing_token = $2",
    )
    .bind(fixture.old_instance)
    .bind(lease.fencing_token)
    .fetch_one(&app_pool)
    .await
    .expect("application RLS query");
    assert_eq!(app_visible, 0, "missing actor cannot read project logs");

    let stale = GatewayServiceInstanceLease {
        fencing_token: lease.fencing_token + 1,
        ..lease.clone()
    };
    assert!(matches!(
        store.append_batch(&stale, &owner, batch()).await,
        Err(GatewayServiceLogStoreError::StaleLease)
    ));
    let wrong_owner = GatewayServiceOwner::new(lease.owner_host_id.clone(), Uuid::new_v4())
        .expect("wrong owner identity");
    assert!(matches!(
        store.append_batch(&lease, &wrong_owner, batch()).await,
        Err(GatewayServiceLogStoreError::StaleLease)
    ));
    let disabled_revision = Uuid::new_v4();
    insert_disabled_service_revision(&pool, disabled_revision, fixture.gateway, fixture.project)
        .await;
    let disabled_lease = GatewayServiceInstanceLease {
        identity: GatewayServiceIdentity {
            revision_id: disabled_revision,
            ..lease.identity
        },
        ..lease.clone()
    };
    assert!(matches!(
        store.append_batch(&disabled_lease, &owner, batch()).await,
        Err(GatewayServiceLogStoreError::Disabled)
    ));
    let expired_fixture = seed_gateway_with_instance_state(
        &pool,
        &format!("log-expired-{}", Uuid::new_v4()),
        "enabled",
        "published",
        "ready",
        true,
        None,
    )
    .await;
    let expired_identity = GatewayServiceIdentity {
        instance_id: expired_fixture.old_instance,
        gateway_id: expired_fixture.gateway,
        revision_id: expired_fixture.old_service,
    };
    let expired_lease = targets
        .get_service_instance(expired_identity)
        .await
        .expect("expired service instance lookup")
        .expect("expired fixture instance");
    let expired_owner = GatewayServiceOwner::new(
        expired_lease.owner_host_id.clone(),
        expired_lease.owner_uuid,
    )
    .expect("expired fixture owner");
    assert!(matches!(
        store
            .append_batch(&expired_lease, &expired_owner, batch())
            .await,
        Err(GatewayServiceLogStoreError::StaleLease)
    ));
}
