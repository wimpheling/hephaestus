//! Actual PUBLIC107 parent-first fixture waiting and rollback; no daemon IO.
use super::{UpdateAdmissionInstance, seed_active_run};
use heph_run::{RunKind, StartRun};
use runtime_types::{
    AgentAttachmentId, AgentInstanceId, AgentInstanceRevisionId, CommandId, ReleaseAgentId,
    ReleaseId, RunId,
};
use sqlx::{PgPool, migrate::Migrator, postgres::PgPoolOptions};
use std::{borrow::Cow, time::Duration};
#[path = "seed_tests/graph.rs"]
mod graph;
const MIGRATIONS: Migrator = sqlx::migrate!("../../migrations");
async fn instance(pool: &PgPool) -> UpdateAdmissionInstance {
    let checkpoint = Migrator {
        migrations: Cow::Owned(
            MIGRATIONS
                .iter()
                .filter(|m| m.version <= 107)
                .cloned()
                .collect(),
        ),
        ..Migrator::DEFAULT
    };
    checkpoint.run(pool).await.unwrap();
    let command = StartRun {
        command_id: CommandId::new(),
        run_id: RunId::new(),
        instance_id: AgentInstanceId::new(),
        instance_revision_id: AgentInstanceRevisionId::new(),
        release_id: ReleaseId::new(),
        release_agent_id: ReleaseAgentId::new(),
        attachment_id: Some(AgentAttachmentId::new()),
        kind: RunKind::Normal,
        requires_state: true,
    };
    graph::seed_instance(pool, &command).await;
    sqlx::query("UPDATE agent_instances SET run_gate_open=false WHERE id=$1")
        .bind(command.instance_id.as_uuid())
        .execute(pool)
        .await
        .unwrap();
    UpdateAdmissionInstance {
        instance_id: command.instance_id.as_uuid(),
        revision_id: command.instance_revision_id.as_uuid(),
        release_id: command.release_id.as_uuid(),
        release_agent_id: command.release_agent_id.as_uuid(),
        attachment_id: command.attachment_id.unwrap().as_uuid(),
    }
}
async fn snapshot(pool: &PgPool, instance: uuid::Uuid) -> serde_json::Value {
    sqlx::query_scalar("SELECT jsonb_build_object('instance',to_jsonb(consumer),'runs',(SELECT COALESCE(jsonb_agg(to_jsonb(execution) ORDER BY execution.id),'[]'::jsonb) FROM runs execution WHERE execution.instance_id=consumer.id)) FROM agent_instances consumer WHERE consumer.id=$1")
 .bind(instance).fetch_one(pool).await.unwrap()
}
#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PUBLIC107 PostgreSQL"]
async fn blocked_parent_waits_before_insert_then_commits_run_and_gate_atomically(pool: PgPool) {
    let instance = instance(&pool).await;
    let before = snapshot(&pool, instance.instance_id).await;
    assert_eq!(before["instance"]["run_gate_open"], false);
    assert_eq!(before["runs"], serde_json::json!([]));
    // A single dedicated backend makes the observed waiter the actual seed.
    let seed_pool = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(pool.connect_options().as_ref().clone())
        .await
        .unwrap();
    let seed_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&seed_pool)
        .await
        .unwrap();
    let mut blocker = pool.begin().await.unwrap();
    let blocker_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *blocker)
        .await
        .unwrap();
    sqlx::query("SELECT id FROM agent_instances WHERE id=$1 FOR UPDATE")
        .bind(instance.instance_id)
        .fetch_one(&mut *blocker)
        .await
        .unwrap();
    let copy = seed_pool.clone();
    let operation = tokio::spawn(async move { seed_active_run(&copy, &instance).await });
    wait_for_seed_parent(&pool, seed_pid, blocker_pid).await;
    assert_eq!(snapshot(&pool, instance.instance_id).await, before);
    // The child has not been inserted/locked while the seed waits on its parent.
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM runs WHERE instance_id=$1")
        .bind(instance.instance_id)
        .fetch_one(&mut *blocker)
        .await
        .unwrap();
    assert_eq!(count, 0);
    blocker.commit().await.unwrap();
    let run = tokio::time::timeout(Duration::from_secs(3), operation)
        .await
        .unwrap()
        .unwrap();
    let after = snapshot(&pool, instance.instance_id).await;
    assert_eq!(after["instance"]["run_gate_open"], true);
    assert_eq!(after["runs"].as_array().unwrap().len(), 1);
    assert_eq!(after["runs"][0]["id"], run.to_string());
    assert_eq!(after["runs"][0]["state"], "running");
    assert_eq!(after["runs"][0]["requires_state"], true);
    seed_pool.close().await;
}

async fn wait_for_seed_parent(pool: &PgPool, seed_pid: i32, blocker_pid: i32) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let waiting: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM pg_stat_activity
                 WHERE pid=$1 AND state='active' AND wait_event_type='Lock'
                   AND $2=ANY(pg_blocking_pids(pid))
                   AND query='SELECT id FROM agent_instances WHERE id=$1 FOR UPDATE')",
            )
            .bind(seed_pid)
            .bind(blocker_pid)
            .fetch_one(pool)
            .await
            .unwrap();
            if waiting {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("seed must actually wait on its held parent before inserting the child");
}
#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PUBLIC107 PostgreSQL"]
async fn permanently_removed_parent_denies_seed_and_rolls_back_every_fixture_change(pool: PgPool) {
    let instance = instance(&pool).await;
    sqlx::query("UPDATE agent_instances SET state='removed',removed_at=now() WHERE id=$1")
        .bind(instance.instance_id)
        .execute(&pool)
        .await
        .unwrap();
    let before = snapshot(&pool, instance.instance_id).await;
    let copy = pool.clone();
    let failure = tokio::spawn(async move { seed_active_run(&copy, &instance).await })
        .await
        .unwrap_err();
    assert!(failure.is_panic());
    let diagnostic = failure.into_panic().downcast::<String>().unwrap();
    assert!(diagnostic.contains("persist active normal update-drain fixture"));
    assert!(diagnostic.contains("instance is closed to new work"));
    assert_eq!(snapshot(&pool, instance.instance_id).await, before);
}
