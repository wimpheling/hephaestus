use mailbox_dispatch::MailboxDispatchStore;
use mailbox_postgres::PostgresMailboxRepository;
use run_domain::{FreshRunExecutionProfile, LegacyVmPlacementScope};
use sqlx::PgPool;
use vm_trait::VmProviderOwnerScope;

use super::support;

fn owner() -> LegacyVmPlacementScope {
    LegacyVmPlacementScope::new(
        VmProviderOwnerScope::new(
            uuid::Uuid::new_v4().to_string(),
            "mailbox-placement-host".into(),
        )
        .unwrap(),
    )
    .unwrap()
}

async fn setup(pool: &PgPool) -> mailbox_dispatch::MailboxDispatchCommand {
    assert_eq!(
        std::env::var("HEPHAESTUS_PRIVATE_RECIPE_MIGRATIONS").as_deref(),
        Ok("1")
    );
    let command = support::setup(pool, true).await;
    sqlx::raw_sql(include_str!(
        "/var/tmp/0108_legacy_scalar_compatibility.sql"
    ))
    .execute(pool)
    .await
    .unwrap();
    sqlx::raw_sql(include_str!("/var/tmp/0108_run_profile_lifecycle.sql"))
        .execute(pool)
        .await
        .unwrap();
    sqlx::raw_sql(include_str!("/var/tmp/0108_legacy_vm_placement.sql"))
        .execute(pool)
        .await
        .unwrap();
    command
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable private Legacy placement PostgreSQL"]
async fn accepted_mailbox_birth_stamps_exact_unconsumed_plan_atomically(pool: PgPool) {
    let command = setup(&pool).await;
    let scope = owner();
    let store = PostgresMailboxRepository::new(pool.clone())
        .with_execution_profile(FreshRunExecutionProfile::LegacyScalar)
        .with_legacy_vm_placement(scope.clone());
    let run = store.claim_dispatch(&command).await.unwrap().unwrap();
    let exact: bool = sqlx::query_scalar("SELECT execution.legacy_vm_placement_required AND execution.vm_id IS NULL AND execution.vm_provider_namespace IS NULL AND plan.consumed_at IS NULL AND plan.provider_namespace=$2 AND plan.host_id=$3 AND plan.vm_id=execution.id::text AND plan.event_id=$4 AND plan.actor_id IS NULL AND plan.request_id IS NULL AND plan.producer_id=attempt.id AND legacy_vm_placement_matches(execution,plan) FROM runs execution JOIN run_legacy_vm_placements plan ON plan.run_id=execution.id JOIN mailbox_delivery_attempts attempt ON attempt.run_id=execution.id WHERE execution.id=$1")
        .bind(run.run_id.as_uuid()).bind(scope.provider_scope().namespace().parse::<uuid::Uuid>().unwrap())
        .bind(scope.provider_scope().host_id()).bind(command.event_id.as_uuid()).fetch_one(&pool).await.unwrap();
    assert!(exact);
    assert!(store.claim_dispatch(&command).await.unwrap().is_none());
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SET LOCAL ROLE hephaestus_worker")
        .execute(&mut *tx)
        .await
        .unwrap();
    assert!(
        sqlx::query("UPDATE runs SET state='leasing_volume' WHERE id=$1")
            .bind(run.run_id.as_uuid())
            .execute(&mut *tx)
            .await
            .is_err()
    );
    tx.rollback().await.unwrap();
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SET LOCAL ROLE hephaestus_worker")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("SELECT consume_legacy_vm_placement($1,$2,$3,$4,'provision')")
        .bind(run.run_id.as_uuid())
        .bind(run.command_id.as_uuid())
        .bind(
            scope
                .provider_scope()
                .namespace()
                .parse::<uuid::Uuid>()
                .unwrap(),
        )
        .bind(scope.provider_scope().host_id())
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let consumed: bool = sqlx::query_scalar("SELECT consumed_at IS NOT NULL AND consumption_kind='provision' FROM run_legacy_vm_placements WHERE run_id=$1")
        .bind(run.run_id.as_uuid()).fetch_one(&pool).await.unwrap();
    assert!(consumed);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable private Legacy placement PostgreSQL"]
async fn missing_explicit_profile_does_not_claim_or_stamp_accepted_event(pool: PgPool) {
    let command = setup(&pool).await;
    let store = PostgresMailboxRepository::new(pool.clone()).with_legacy_vm_placement(owner());
    assert!(store.claim_dispatch(&command).await.is_err());
    let unchanged: bool = sqlx::query_scalar("SELECT disposition='eligible' AND logical_attempt_count=0 AND NOT EXISTS(SELECT 1 FROM mailbox_delivery_attempts WHERE event_id=$1) AND NOT EXISTS(SELECT 1 FROM run_legacy_vm_placements) FROM mailbox_deliveries WHERE event_id=$1")
        .bind(command.event_id.as_uuid()).fetch_one(&pool).await.unwrap();
    assert!(unchanged);
}
