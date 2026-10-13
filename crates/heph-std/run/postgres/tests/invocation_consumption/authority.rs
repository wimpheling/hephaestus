use authz_domain::{ObjectRef, ObjectType, Permission};
use run_orchestrator::RunRuntimeCatalog;
use sqlx::PgPool;
use uuid::Uuid;

use super::fixture::Fixture;

async fn withdraw(pool: &PgPool, project: Uuid, user: Uuid) {
    sqlx::query("DELETE FROM project_maintainers WHERE project_id=$1 AND user_id=$2")
        .bind(project)
        .bind(user)
        .execute(pool)
        .await
        .unwrap();
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and private117"]
async fn source_withdrawal_denies_first_consumption_replay_and_catalogue(pool: PgPool) {
    let h = Fixture::new(&pool).await;
    let actor = &h.producer.activation.actor;
    let source = h.producer.activation.fixture.project;
    withdraw(&pool, source, actor.user_id.as_uuid()).await;
    assert!(!h.producer.activation.can_use(actor).await);
    let before = h.rows().await;
    assert!(h.consume().await.is_err());
    assert_eq!(h.rows().await, before);
    sqlx::query("INSERT INTO project_maintainers(project_id,user_id) VALUES($1,$2)")
        .bind(source)
        .bind(actor.user_id.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    let run = h.consume().await.unwrap().run;
    withdraw(&pool, source, actor.user_id.as_uuid()).await;
    assert!(!h.producer.activation.can_use(actor).await);
    let before = h.rows().await;
    assert!(h.consume().await.is_err());
    assert!(h.repository().load_runtime(&run).await.is_err());
    assert_eq!(
        h.repository()
            .inspect_consumed_invocation(run.id)
            .await
            .unwrap(),
        run
    );
    assert_eq!(h.rows().await, before);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and private117"]
async fn withdrawn_execute_and_inactive_original_caller_cannot_be_borrowed(pool: PgPool) {
    let h = Fixture::new(&pool).await;
    let actor = &h.producer.activation.actor;
    let project = h.producer.activation.fixture.consuming_project;
    withdraw(&pool, project, actor.user_id.as_uuid()).await;
    assert!(
        !h.producer
            .activation
            .permission(
                actor,
                Permission::CanExecute,
                ObjectRef::new(ObjectType::AgentInstance, h.start.instance_id.as_uuid())
            )
            .await
    );
    let before = h.rows().await;
    assert!(h.consume().await.is_err());
    assert_eq!(h.rows().await, before);
    sqlx::query("INSERT INTO project_maintainers(project_id,user_id) VALUES($1,$2)")
        .bind(project)
        .bind(actor.user_id.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE users SET status='suspended' WHERE id=$1")
        .bind(actor.user_id.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    let before = h.rows().await;
    assert!(h.consume().await.is_err());
    assert_eq!(h.rows().await, before);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and private117"]
async fn admitted_remove_keeps_immutable_consumption_but_blocks_live_execution(pool: PgPool) {
    let h = Fixture::new(&pool).await;
    let run = h.consume().await.unwrap().run;
    let manager = crate::support::identity(h.producer.activation.fixture.outsider);
    assert_ne!(manager.user_id, h.producer.activation.actor.user_id);
    sqlx::query("INSERT INTO project_maintainers(project_id,user_id) VALUES($1,$2)")
        .bind(h.producer.activation.fixture.consuming_project)
        .bind(manager.user_id.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    let snapshot = h
        .producer
        .activation
        .recipe
        .inspect(&manager, h.producer.activation.intent.id())
        .await
        .unwrap();
    h.producer
        .activation
        .recipe
        .admit_remove(
            &manager,
            crate::fixtures::remove(
                &manager,
                h.producer.activation.intent.id(),
                snapshot.version,
            ),
        )
        .await
        .unwrap();
    let before = h.rows().await;
    assert!(h.consume().await.is_err());
    assert!(h.repository().load_runtime(&run).await.is_err());
    assert_eq!(
        h.repository()
            .inspect_consumed_invocation(run.id)
            .await
            .unwrap(),
        run
    );
    let immutable: bool = sqlx::query_scalar(
        "SELECT run_profile_producer_matches(execution) FROM runs execution WHERE id=$1",
    )
    .bind(run.id.as_uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(immutable);
    assert_eq!(h.rows().await, before);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and private117"]
async fn current_gate_closure_denies_without_reopening_or_mutating_receipt(pool: PgPool) {
    let h = Fixture::new(&pool).await;
    let run = h.consume().await.unwrap().run;
    sqlx::query("UPDATE agent_instances SET run_gate_open=false,version=version+1 WHERE id=$1")
        .bind(h.start.instance_id.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    let before = h.rows().await;
    assert!(h.consume().await.is_err());
    assert!(h.repository().load_runtime(&run).await.is_err());
    assert_eq!(
        h.repository()
            .inspect_consumed_invocation(run.id)
            .await
            .unwrap(),
        run
    );
    assert_eq!(h.rows().await, before);
}
