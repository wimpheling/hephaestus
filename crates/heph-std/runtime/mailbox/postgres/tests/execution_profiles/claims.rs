use mailbox_dispatch::MailboxDispatchStore;
use mailbox_postgres::PostgresMailboxRepository;
use run_domain::FreshRunExecutionProfile;
use sqlx::PgPool;

use super::support;

#[sqlx::test(migrations = false)]
#[ignore = "requires actual disposable PostgreSQL"]
async fn default_mailbox_constructor_retains_public107_sql(pool: PgPool) {
    let command = support::setup(&pool, false).await;
    let store = PostgresMailboxRepository::new(pool.clone());
    let run = store.claim_dispatch(&command).await.unwrap().unwrap();
    assert!(store.claim_dispatch(&command).await.unwrap().is_none());
    let exact: bool =
        sqlx::query_scalar("SELECT command_id=$2 FROM mailbox_delivery_attempts WHERE run_id=$1")
            .bind(run.run_id.as_uuid())
            .bind(run.command_id.as_uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(exact);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires actual private profile schema"]
async fn explicit_legacy_mailbox_commits_profile_with_exact_accepted_attempt(pool: PgPool) {
    let command = support::setup(&pool, true).await;
    let store = PostgresMailboxRepository::new(pool.clone())
        .with_execution_profile(FreshRunExecutionProfile::LegacyScalar);
    let run = store.claim_dispatch(&command).await.unwrap().unwrap();
    let exact: bool = sqlx::query_scalar("SELECT execution.execution_profile='legacy_scalar' AND NOT execution.vm_plan_admission_required AND execution.vm_binding_kind IS NULL AND attempt.event_id=$2 AND attempt.command_id=execution.command_id AND attempt.instance_revision_id=execution.instance_revision_id AND attempt.target_commit IS NOT NULL FROM runs execution JOIN mailbox_delivery_attempts attempt ON attempt.run_id=execution.id WHERE execution.id=$1")
        .bind(run.run_id.as_uuid()).bind(command.event_id.as_uuid()).fetch_one(&pool).await.unwrap();
    assert!(exact);
    assert!(store.claim_dispatch(&command).await.unwrap().is_none());
}

#[sqlx::test(migrations = false)]
#[ignore = "requires actual private profile schema"]
async fn owned_mailbox_rejected_before_any_claim_or_producer_mutation(pool: PgPool) {
    let command = support::setup(&pool, true).await;
    let store = PostgresMailboxRepository::new(pool.clone())
        .with_execution_profile(FreshRunExecutionProfile::OwnedCanonical);
    let error = store.claim_dispatch(&command).await.unwrap_err();
    assert!(error.0.contains("unsupported"));
    let unchanged: bool = sqlx::query_scalar("SELECT disposition='eligible' AND logical_attempt_count=0 AND dispatch_sequence IS NULL AND NOT EXISTS(SELECT 1 FROM mailbox_delivery_attempts WHERE event_id=$1) AND NOT EXISTS(SELECT 1 FROM runs) FROM mailbox_deliveries WHERE event_id=$1")
        .bind(command.event_id.as_uuid()).fetch_one(&pool).await.unwrap();
    assert!(unchanged);
    // The original accepted event can still use the explicit legacy producer;
    // unsupported owned dispatch neither drops work nor borrows a new principal.
    PostgresMailboxRepository::new(pool.clone())
        .with_execution_profile(FreshRunExecutionProfile::LegacyScalar)
        .claim_dispatch(&command)
        .await
        .unwrap()
        .unwrap();
}
