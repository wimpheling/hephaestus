use super::fixture::Core;
use identity_domain::AuthenticatedIdentity;
use recipe_application::{
    EffectClaim, OwnedReadyRetentionRepository, RetainedExternalCompletion,
    RetainedExternalVerificationPlan,
};
use serde_json::Value;
use std::time::Duration;

pub async fn snapshot(core: &Core, claim: &EffectClaim) -> Value {
    sqlx::query_scalar("SELECT jsonb_build_object(
      'reopened',(SELECT jsonb_agg(to_jsonb(row) ORDER BY id) FROM recipe_owned_ready_reopenings row),
      'transitions',(SELECT jsonb_agg(to_jsonb(row) ORDER BY id) FROM recipe_execution_transitions row),
      'heads',(SELECT jsonb_agg(to_jsonb(row) ORDER BY id) FROM agent_instance_state_volumes row),
      'resources',(SELECT jsonb_agg(to_jsonb(row) ORDER BY deployment_id,resource_name) FROM recipe_deployment_resources row),
      'deployment',(SELECT to_jsonb(row) FROM recipe_deployments row WHERE id=$1),
      'events',(SELECT jsonb_agg(to_jsonb(row) ORDER BY id) FROM application_events row),
      'outbox',(SELECT jsonb_agg(to_jsonb(row) ORDER BY event_id) FROM product_event_outbox row))")
        .bind(claim.deployment_id.as_uuid()).fetch_one(&core.admin).await.unwrap()
}

pub async fn force_completion_rollback(
    core: &Core,
    identity: &AuthenticatedIdentity,
    plan: &RetainedExternalVerificationPlan,
) {
    let before = snapshot(core, &plan.claim).await;
    let mut lock = core.admin.begin().await.unwrap();
    sqlx::query("LOCK TABLE public.recipe_execution_transitions IN SHARE MODE")
        .execute(&mut *lock)
        .await
        .unwrap();
    let actor = core.actor.clone();
    let identity = identity.clone();
    let claim = plan.claim.clone();
    let plan = plan.clone();
    let task = tokio::spawn(async move {
        actor
            .complete_retained_external_verification(
                &identity,
                &plan,
                RetainedExternalCompletion::Completed,
            )
            .await
    });
    // The actual production normal-completion statement, not its preceding
    // reopening append, must be waiting on this exact table's INSERT lock.
    let observed = tokio::time::timeout(Duration::from_secs(1), observe_insert(core)).await;
    let outcome = task.await;
    let release = lock.rollback().await;
    release.unwrap(); // Always release the admin lock before asserting either result.
    let pid = observed.unwrap().unwrap();
    tokio::time::timeout(Duration::from_secs(1), observe_rollback(core, pid))
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        outcome.unwrap(),
        Err(recipe_application::DeploymentError::Repository)
    ));
    // A definite statement error aborted the actor transaction. Its actual held
    // backend must finish rollback before snapshots or native custody release.
    // Do not infer awaited rollback from SQLx transaction Drop.
    assert_eq!(snapshot(core, &claim).await, before);
}

async fn observe_insert(core: &Core) -> Result<i32, sqlx::Error> {
    loop {
        let pids: Vec<i32> = sqlx::query_scalar("SELECT activity.pid
          FROM pg_catalog.pg_stat_activity activity
          JOIN pg_catalog.pg_locks waiting ON waiting.pid=activity.pid
          WHERE activity.application_name=$1 AND activity.state='active'
            AND activity.datid=(SELECT oid FROM pg_catalog.pg_database WHERE datname=current_database())
            AND activity.wait_event_type='Lock'
            AND activity.query LIKE 'INSERT INTO recipe_execution_transitions (%'
            AND waiting.relation='public.recipe_execution_transitions'::regclass
            AND waiting.mode='RowExclusiveLock' AND NOT waiting.granted")
            .bind(&core.app_name).fetch_all(&core.admin).await?;
        match pids.as_slice() {
            [pid] => return Ok(*pid),
            [] => {}
            _ => {
                return Err(sqlx::Error::Protocol(
                    "ambiguous normal-completion backend".into(),
                ));
            }
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn observe_rollback(core: &Core, pid: i32) -> Result<(), sqlx::Error> {
    loop {
        let ended: bool = sqlx::query_scalar(
            "SELECT NOT EXISTS(
          SELECT 1 FROM pg_catalog.pg_stat_activity WHERE pid=$1 AND xact_start IS NOT NULL)",
        )
        .bind(pid)
        .fetch_one(&core.admin)
        .await?;
        if ended {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
