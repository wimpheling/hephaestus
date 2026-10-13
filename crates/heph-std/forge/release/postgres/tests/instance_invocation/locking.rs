use identity_domain::RequestId;
use release_service::{InstanceExecutionError, InstanceExecutionService};
use sqlx::PgPool;

use super::fixture::{Invocation, code};

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and private115"]
async fn concurrent_same_command_commits_one_original_run_and_start_outbox(pool: PgPool) {
    let h = Invocation::new(&pool).await;
    let mut retry = h.activation.actor.clone();
    retry.request_id = RequestId::new();
    let first = h.service();
    let second = h.service();
    let (a, b) = tokio::join!(
        first.invoke_instance(&h.activation.actor, h.command),
        second.invoke_instance(&retry, h.command)
    );
    assert_eq!(a.unwrap(), b.unwrap());
    let counts:(i64,i64,i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM agent_instance_execution_invocations WHERE instance_id=$1),(SELECT count(*) FROM runs WHERE instance_id=$1),(SELECT count(*) FROM outbox WHERE aggregate_id=$2 AND event_type='run.start.v1'),(SELECT count(*) FROM agent_instance_execution_invocation_audits WHERE invocation_id=$3)")
        .bind(h.command.instance_id().as_uuid()).bind(h.command.run_id().as_uuid()).bind(h.command.invocation_id().as_uuid()).fetch_one(&pool).await.unwrap();
    assert_eq!(counts, (1, 1, 1, 2));
    let before = h.rows().await;
    first.invoke_instance(&retry, h.command).await.unwrap();
    assert_eq!(h.rows().await, before);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and private115"]
async fn held_late_instance_is_uncertain_without_deadlock_or_replacement(pool: PgPool) {
    let h = Invocation::new(&pool).await;
    let before = h.rows().await;
    let mut blocker = pool.begin().await.unwrap();
    sqlx::query("SELECT id FROM agent_instances WHERE id=$1 FOR UPDATE")
        .bind(h.command.instance_id().as_uuid())
        .execute(&mut *blocker)
        .await
        .unwrap();
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        h.service().invoke_instance(&h.activation.actor, h.command),
    )
    .await
    .expect("late NOWAIT must not deadlock");
    assert!(matches!(
        result,
        Err(InstanceExecutionError::OutcomeUncertain)
    ));
    let mut tx = h.transaction().await;
    let error =
        sqlx::query("SELECT * FROM write_qualified_instance_invocation($1,$2,$3,$4,$5,$6,$7,$8)")
            .bind(h.command.invocation_id().as_uuid())
            .bind(h.command.command_key().as_bytes().as_slice())
            .bind(h.command.run_id().as_uuid())
            .bind(h.command.start_command_id().as_uuid())
            .bind(h.command.instance_id().as_uuid())
            .bind(h.command.expected_revision_id().as_uuid())
            .bind(&h.activation.config_wire)
            .bind(authz_postgres::AUTHORIZATION_MODEL_VERSION)
            .execute(&mut *tx)
            .await
            .unwrap_err();
    assert_eq!(
        code(&error).as_deref(),
        Some("55P03"),
        "transient held state, not absence: {error:?}"
    );
    tx.rollback().await.unwrap();
    assert_eq!(h.rows().await, before);
    blocker.rollback().await.unwrap();
    h.service()
        .invoke_instance(&h.activation.actor, h.command)
        .await
        .unwrap();
}
