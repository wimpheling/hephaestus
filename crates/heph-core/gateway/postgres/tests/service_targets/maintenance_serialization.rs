//! maintenance serialization scenario.

use super::support::{
    batch_with_sequence, named_worker_pool, seed_gateway_with_instance_state, test_pool,
    wait_for_blocked_workers, worker_pool,
};
use gateway_domain::{
    GatewayServiceIdentity, GatewayServiceLogMaintenance, GatewayServiceLogMaintenancePolicy,
    GatewayServiceLogStore, GatewayServiceOwner, GatewayServiceTargetStore,
};
use gateway_postgres::{PostgresGatewayServiceLogStore, PostgresGatewayServiceTargets};
use serial_test::serial;
use std::time::Duration;
use uuid::Uuid;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
async fn service_log_append_and_maintenance_serialize_without_deadlock() {
    let Some(pool) = test_pool().await else {
        return;
    };
    for maintenance_first in [true, false] {
        let worker = worker_pool().await;
        let fixture = seed_gateway_with_instance_state(
            &pool,
            &format!("log-lock-order-{}-{}", maintenance_first, Uuid::new_v4()),
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
            .expect("lock-order instance lookup")
            .expect("lock-order instance");
        let owner = GatewayServiceOwner::new(lease.owner_host_id.clone(), lease.owner_uuid)
            .expect("lock-order owner");
        let setup_store = PostgresGatewayServiceLogStore::new(worker);
        setup_store
            .append_batch(&lease, &owner, batch_with_sequence(0))
            .await
            .expect("seed lock-order payload");
        sqlx::query(
            "UPDATE gateway_service_log_chunks
                SET stored_at = clock_timestamp() - interval '25 hours'
              WHERE instance_id = $1 AND fencing_token = $2 AND sequence = 0",
        )
        .bind(fixture.old_instance)
        .bind(lease.fencing_token)
        .execute(&pool)
        .await
        .expect("age lock-order payload");

        let suffix = Uuid::new_v4().simple().to_string();
        let maintenance_name = format!("log-maintenance-{suffix}");
        let append_name = format!("log-append-{suffix}");
        let maintenance_pool = named_worker_pool(&maintenance_name).await;
        let append_pool = named_worker_pool(&append_name).await;
        let maintenance_store = PostgresGatewayServiceLogStore::new(maintenance_pool.clone());
        let append_store = PostgresGatewayServiceLogStore::new(append_pool.clone());
        let mut holder = pool.begin().await.expect("lock-order holder transaction");
        sqlx::query("SELECT set_config('application_name', $1, false)")
            .bind(format!("log-holder-{suffix}"))
            .execute(&mut *holder)
            .await
            .expect("name lock holder");
        let holder_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *holder)
            .await
            .expect("lock holder pid");
        sqlx::query(
            "SELECT project_id
               FROM gateway_service_log_project_usage
              WHERE project_id = $1
              FOR UPDATE",
        )
        .bind(fixture.project)
        .fetch_one(&mut *holder)
        .await
        .expect("hold project usage lock");

        let project_id = fixture.project;
        let append_lease = lease.clone();
        let append_owner = owner.clone();
        let maintenance_task = async move {
            maintenance_store
                .maintain_project(project_id, GatewayServiceLogMaintenancePolicy::new(8, 8))
                .await
        };
        let append_task = async move {
            append_store
                .append_batch(&append_lease, &append_owner, batch_with_sequence(1))
                .await
        };
        let maintenance_join;
        let append_join;
        if maintenance_first {
            maintenance_join = tokio::spawn(maintenance_task);
            wait_for_blocked_workers(&pool, holder_pid, &[&maintenance_name]).await;
            append_join = tokio::spawn(append_task);
        } else {
            append_join = tokio::spawn(append_task);
            wait_for_blocked_workers(&pool, holder_pid, &[&append_name]).await;
            maintenance_join = tokio::spawn(maintenance_task);
        }
        wait_for_blocked_workers(&pool, holder_pid, &[&maintenance_name, &append_name]).await;
        holder.commit().await.expect("release lock holder");

        let maintenance_result = tokio::time::timeout(Duration::from_secs(10), maintenance_join)
            .await
            .expect("maintenance completes after release")
            .expect("maintenance task joins")
            .expect("maintenance succeeds");
        let append_result = tokio::time::timeout(Duration::from_secs(10), append_join)
            .await
            .expect("append completes after release")
            .expect("append task joins")
            .expect("append succeeds");
        assert_eq!(maintenance_result.expired_chunks, 1);
        assert_eq!(maintenance_result.expired_bytes, 18);
        assert_eq!(append_result.accepted_chunks, 1);

        let (rows, bytes, acknowledged): (i64, i64, i64) = sqlx::query_as(
            "SELECT count(*)::bigint,
                    coalesce(sum(octet_length(chunks.bytes)), 0)::bigint,
                    epochs.acknowledged_through
               FROM gateway_service_log_chunks chunks
               JOIN gateway_service_log_epochs epochs
                 ON epochs.instance_id = chunks.instance_id
                AND epochs.fencing_token = chunks.fencing_token
              WHERE chunks.project_id = $1
              GROUP BY epochs.acknowledged_through",
        )
        .bind(fixture.project)
        .fetch_one(&pool)
        .await
        .expect("consistent lock-order log totals");
        let usage: (i64, i64) = sqlx::query_as(
            "SELECT retained_chunks, retained_bytes
               FROM gateway_service_log_project_usage
              WHERE project_id = $1",
        )
        .bind(fixture.project)
        .fetch_one(&pool)
        .await
        .expect("consistent lock-order usage");
        assert_eq!((rows, bytes), (usage.0, usage.1));
        assert_eq!((rows, bytes, acknowledged), (1, 18, 1));
        let replay = setup_store
            .append_batch(&lease, &owner, batch_with_sequence(0))
            .await
            .expect("replay after lock-order maintenance");
        assert_eq!(replay.accepted_chunks, 0);
        let sequence_zero: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM gateway_service_log_chunks
              WHERE instance_id = $1 AND fencing_token = $2 AND sequence = 0",
        )
        .bind(fixture.old_instance)
        .bind(lease.fencing_token)
        .fetch_one(&pool)
        .await
        .expect("lock-order replay row count");
        assert_eq!(sequence_zero, 0);

        maintenance_pool.close().await;
        append_pool.close().await;
    }
}
