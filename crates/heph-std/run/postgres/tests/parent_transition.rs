//! Public107 parent-before-Run locking; actual disposable `PostgreSQL` only.

#[path = "postgres/support.rs"]
#[allow(dead_code)] // Reuse the established exact released revision/instance fixture.
mod support;

use run_domain::{RunKind, RunState, StartRun};
use run_orchestrator::{RepositoryError, RunRepository};
use run_postgres::PgRunRepository;
use runtime_types::{
    AgentAttachmentId, AgentInstanceId, AgentInstanceRevisionId, CommandId, ReleaseAgentId,
    ReleaseId, RunId,
};
use sqlx::{PgPool, Postgres, Transaction, postgres::PgPoolOptions};
use std::time::Duration;
use tokio::task::JoinHandle;

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL at public107"]
async fn transition_waits_for_parent_without_locking_run_and_commits_event(pool: PgPool) {
    let fixture = Fixture::new(&pool).await;
    assert_old_order_conflict(&fixture, &pool).await;
    let (mut blocker, operation) = fixture.blocked_transition(&pool).await;
    // The waiting transition must not already own the child. The parent holder
    // can now lock that exact child immediately, proving the cycle is removed.
    sqlx::query("SELECT id FROM runs WHERE id=$1 FOR UPDATE NOWAIT")
        .bind(fixture.command.run_id.as_uuid())
        .fetch_one(&mut *blocker)
        .await
        .expect("parent holder can lock child while transition waits");
    assert_eq!(
        snapshot(&pool, fixture.command.run_id).await,
        ("queued".to_owned(), 0, 1)
    );
    blocker.commit().await.unwrap();
    let result = tokio::time::timeout(Duration::from_secs(5), operation)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(result.state, RunState::LeasingVolume);
    assert_eq!(
        snapshot(&pool, fixture.command.run_id).await,
        ("leasing_volume".to_owned(), 1, 2)
    );
    let event_complete: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM run_events WHERE run_id=$1 AND event_type='run.leasing_volume')
          AND EXISTS(SELECT 1 FROM application_events event JOIN product_event_outbox outbox
            ON outbox.event_id=event.id WHERE event.aggregate_type='run' AND event.aggregate_id=$1
            AND event.scope_kind='run' AND event.scope_id=$1
            AND event.event_type='run.changed' AND event.change_kind='state_changed'
            AND event.safe_state='running')",
    ).bind(fixture.command.run_id.as_uuid()).fetch_one(&pool).await.unwrap();
    assert!(
        event_complete,
        "Run progress and committed event/outbox are atomic"
    );
    fixture.worker.close().await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL at public107"]
async fn closure_committed_while_waiting_still_denies_progress_without_events(pool: PgPool) {
    let fixture = Fixture::new(&pool).await;
    let before = snapshot(&pool, fixture.command.run_id).await;
    let (mut blocker, operation) = fixture.blocked_transition(&pool).await;
    sqlx::query("UPDATE agent_instances SET state='removed',removed_at=now() WHERE id=$1")
        .bind(fixture.command.instance_id.as_uuid())
        .execute(&mut *blocker)
        .await
        .unwrap();
    blocker.commit().await.unwrap();
    let error = tokio::time::timeout(Duration::from_secs(5), operation)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    let RepositoryError::Storage(error) = error else {
        panic!("expected unchanged107 admission guard denial");
    };
    let database_error = error.downcast_ref::<sqlx::Error>().unwrap();
    assert_eq!(
        database_error
            .as_database_error()
            .and_then(sqlx::error::DatabaseError::code)
            .as_deref(),
        Some("23000")
    );
    assert_eq!(before, snapshot(&pool, fixture.command.run_id).await);
    fixture.worker.close().await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL at public107"]
async fn missing_run_retains_not_found_and_produces_no_transition(pool: PgPool) {
    sqlx::migrate!("../../../../migrations")
        .run(&pool)
        .await
        .unwrap();
    let repository = PgRunRepository::new(pool.clone());
    let id = RunId::new();
    assert!(
        matches!(repository.transition(id, RunState::LeasingVolume, None, None).await,
        Err(RepositoryError::NotFound(missing)) if missing == id)
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM run_events WHERE run_id=$1")
            .bind(id.as_uuid())
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
}

struct Fixture {
    command: StartRun,
    worker: PgPool,
    worker_pid: i32,
}

async fn assert_old_order_conflict(fixture: &Fixture, pool: &PgPool) {
    let before = snapshot(pool, fixture.command.run_id).await;
    let mut parent = pool.begin().await.unwrap();
    sqlx::query("SELECT id FROM agent_instances WHERE id=$1 FOR UPDATE")
        .bind(fixture.command.instance_id.as_uuid())
        .fetch_one(&mut *parent)
        .await
        .unwrap();
    let mut worker = fixture.worker.begin().await.unwrap();
    sqlx::query("SELECT id FROM runs WHERE id=$1 FOR UPDATE")
        .bind(fixture.command.run_id.as_uuid())
        .fetch_one(&mut *worker)
        .await
        .unwrap();
    // Reproduce the old child-first transition under the actual worker role.
    // The unchanged107 progress trigger must encounter the other parent holder.
    let error = sqlx::query(
        "UPDATE runs SET state='leasing_volume',state_version=state_version+1,updated_at=now()
         WHERE id=$1",
    )
    .bind(fixture.command.run_id.as_uuid())
    .execute(&mut *worker)
    .await
    .unwrap_err();
    assert_eq!(
        error
            .as_database_error()
            .and_then(sqlx::error::DatabaseError::code)
            .as_deref(),
        Some("55P03")
    );
    worker.rollback().await.unwrap();
    parent.rollback().await.unwrap();
    assert_eq!(before, snapshot(pool, fixture.command.run_id).await);
}

impl Fixture {
    async fn new(pool: &PgPool) -> Self {
        sqlx::migrate!("../../../../migrations")
            .run(pool)
            .await
            .unwrap();
        let command = StartRun {
            command_id: CommandId::new(),
            run_id: RunId::new(),
            instance_id: AgentInstanceId::new(),
            instance_revision_id: AgentInstanceRevisionId::new(),
            release_id: ReleaseId::new(),
            release_agent_id: ReleaseAgentId::new(),
            attachment_id: Some(AgentAttachmentId::new()),
            kind: RunKind::Normal,
            requires_state: false,
        };
        support::seed_instance(pool, &command).await;
        support::seed_run_request(pool, &command).await;
        let worker = PgPoolOptions::new()
            .max_connections(1)
            .after_connect(|connection, _| {
                Box::pin(async move {
                    sqlx::query("SET ROLE hephaestus_worker")
                        .execute(connection)
                        .await?;
                    Ok(())
                })
            })
            .connect_with(pool.connect_options().as_ref().clone())
            .await
            .unwrap();
        let repository = PgRunRepository::new(worker.clone());
        repository.create_run(&command).await.unwrap();
        let worker_pid = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&worker)
            .await
            .unwrap();
        Self {
            command,
            worker,
            worker_pid,
        }
    }

    async fn blocked_transition<'a>(
        &self,
        pool: &'a PgPool,
    ) -> (
        Transaction<'a, Postgres>,
        JoinHandle<Result<run_domain::Run, RepositoryError>>,
    ) {
        let mut blocker = pool.begin().await.unwrap();
        let blocker_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *blocker)
            .await
            .unwrap();
        sqlx::query("SELECT id FROM agent_instances WHERE id=$1 FOR UPDATE")
            .bind(self.command.instance_id.as_uuid())
            .fetch_one(&mut *blocker)
            .await
            .unwrap();
        let repository = PgRunRepository::new(self.worker.clone());
        let run_id = self.command.run_id;
        let operation = tokio::spawn(async move {
            repository
                .transition(run_id, RunState::LeasingVolume, None, None)
                .await
        });
        wait_for_parent(pool, self.worker_pid, blocker_pid).await;
        (blocker, operation)
    }
}

async fn wait_for_parent(pool: &PgPool, worker: i32, blocker: i32) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let waiting: bool = sqlx::query_scalar("SELECT $2=ANY(pg_blocking_pids($1))")
                .bind(worker)
                .bind(blocker)
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
    .expect("transition must actually wait on held parent; no sleep-based synchronization");
}

async fn snapshot(pool: &PgPool, run: RunId) -> (String, i64, i64) {
    sqlx::query_as(
        "SELECT state,state_version,(SELECT count(*) FROM run_events WHERE run_id=runs.id)
      FROM runs WHERE id=$1",
    )
    .bind(run.as_uuid())
    .fetch_one(pool)
    .await
    .unwrap()
}
