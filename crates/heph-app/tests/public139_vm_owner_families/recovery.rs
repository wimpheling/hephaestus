//! Genuine public139 inventory only: no provider IO or cleanup receipt is claimed.
use release_service::InstanceExecutionConfiguration;
use run_domain::{CancelRun, RunCleanupHostId, RunState};
use run_orchestrator::{RunCleanupRepository, RunRepository};
use run_postgres::PgRunCleanupRepository;
use runtime_types::{CommandId, RunId};
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;
use vm_trait::{VmId, VmProviderOwnerScope};

use super::{birth, consumptions, original_receipt, prepared};

fn cleanup(pool: &PgPool, config: &InstanceExecutionConfiguration) -> PgRunCleanupRepository {
    let owner = config.provider_scope();
    PgRunCleanupRepository::new(
        pool.clone(),
        RunCleanupHostId::new(owner.namespace().into(), owner.host_id().into()).unwrap(),
    )
    .with_qualified_invocation_configuration(config.clone())
    .with_qualified_queued_invocation_cleanup(config.clone())
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable ordinary public139 PostgreSQL through DATABASE_URL and local Git"]
async fn genuine_queued_intent_and_consumption_stay_redeliverable_then_cancelled_is_inventoried(
    pool: PgPool,
) {
    let original = crate::fixture::Fixture::new(&pool).await;
    let config = crate::fixture::configuration(&original.scope, "test/v1");
    let owner = config.provider_scope();
    let cleanup = cleanup(&original.worker, &config);
    assert!(
        cleanup
            .qualified_invocation_recovery_inventory(owner, None)
            .await
            .unwrap()
            .is_empty()
    );
    let (actor, imported) = prepared::import(&pool, &original, &config).await;
    let (start, runs, scope) = birth::admit(&pool, &original, &actor, &imported, &config).await;
    let receipt = original_receipt(&pool, start.run_id.as_uuid()).await;
    let queued = observation(&pool, start.run_id).await;
    assert_eq!(consumptions(&pool, start.run_id.as_uuid()).await, 0);
    assert!(
        cleanup
            .qualified_invocation_recovery_inventory(owner, None)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        observation(&pool, start.run_id).await,
        queued,
        "RRRO inventory does not consume, close acquisition, prepare a target or release leases"
    );
    // Invoke already admitted the Run: worker consumption is not a second birth.
    assert!(
        !runs
            .create_run_with_vm_plan(&start, &scope, &VmId(start.run_id.to_string()))
            .await
            .unwrap()
            .created
    );
    assert_eq!(consumptions(&pool, start.run_id.as_uuid()).await, 1);
    assert!(
        cleanup
            .qualified_invocation_recovery_inventory(owner, None)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        runs.request_cancel(&CancelRun {
            command_id: CommandId::new(),
            run_id: start.run_id,
            reason: "qualified restart inventory before provider IO".into()
        })
        .await
        .unwrap()
    );
    let cancelled_queued = observation(&pool, start.run_id).await;
    assert_eq!(
        cleanup
            .qualified_invocation_recovery_inventory(owner, None)
            .await
            .unwrap(),
        vec![start.run_id]
    );
    assert_eq!(
        observation(&pool, start.run_id).await,
        cancelled_queued,
        "cancelled queued consumption is observed without inventing a terminal transition"
    );
    runs.transition(start.run_id, RunState::Cancelled, None, None)
        .await
        .unwrap();
    let cancelled = observation(&pool, start.run_id).await;
    assert_eq!(
        cleanup
            .qualified_invocation_recovery_inventory(owner, None)
            .await
            .unwrap(),
        vec![start.run_id]
    );
    assert_eq!(
        cleanup
            .qualified_invocation_recovery_inventory(owner, Some(OffsetDateTime::now_utc()))
            .await
            .unwrap(),
        vec![start.run_id]
    );
    assert_eq!(observation(&pool, start.run_id).await, cancelled);
    assert_eq!(
        original_receipt(&pool, start.run_id.as_uuid()).await,
        receipt
    );
    drop(cleanup);
    drop(runs);
    original.close().await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable ordinary public139 PostgreSQL through DATABASE_URL and local Git"]
async fn actual_queued_original_wrong_owner_configuration_and_nonworker_inventory_are_held(
    pool: PgPool,
) {
    let original = crate::fixture::Fixture::new(&pool).await;
    let config = crate::fixture::configuration(&original.scope, "test/v1");
    let (actor, imported) = prepared::import(&pool, &original, &config).await;
    let (start, runs, _) = birth::admit(&pool, &original, &actor, &imported, &config).await;
    drop(runs);
    let before = observation(&pool, start.run_id).await;
    let reader = cleanup(&original.worker, &config);
    let different = VmProviderOwnerScope::new(
        Uuid::new_v4().to_string(),
        config.provider_scope().host_id().into(),
    )
    .unwrap();
    assert!(
        reader
            .qualified_invocation_recovery_inventory(&different, None)
            .await
            .is_err()
    );
    // The global canonical original cannot disappear behind a config filter.
    let changed_config = crate::fixture::configuration(&original.scope, "test/v2");
    let changed = cleanup(&original.worker, &changed_config);
    assert!(
        changed
            .qualified_invocation_recovery_inventory(changed_config.provider_scope(), None)
            .await
            .is_err()
    );
    let application = cleanup(&original.application, &config);
    assert!(
        application
            .qualified_invocation_recovery_inventory(config.provider_scope(), None)
            .await
            .is_err()
    );
    assert_eq!(observation(&pool, start.run_id).await, before);
    assert_eq!(consumptions(&pool, start.run_id.as_uuid()).await, 0);
    drop(application);
    drop(changed);
    drop(reader);
    original.close().await;
}

async fn observation(pool: &PgPool, run: RunId) -> serde_json::Value {
    sqlx::query_scalar("SELECT jsonb_build_object('run',to_jsonb(execution),'leases',(SELECT COALESCE(jsonb_agg(to_jsonb(lease) ORDER BY lease.id),'[]'::jsonb) FROM agent_instance_volume_leases lease WHERE lease.run_id=execution.id),'targets',(SELECT COALESCE(jsonb_agg(to_jsonb(target)),'[]'::jsonb) FROM run_cleanup_targets target WHERE target.run_id=execution.id),'receipts',(SELECT COALESCE(jsonb_agg(to_jsonb(receipt)),'[]'::jsonb) FROM run_cleanup_receipts receipt WHERE receipt.run_id=execution.id)) FROM runs execution WHERE execution.id=$1")
        .bind(run.as_uuid()).fetch_one(pool).await.unwrap()
}
