use run_domain::StartRun;
use sqlx::PgPool;

pub async fn profile_schema(pool: &PgPool) {
    super::fixture::checkpoint(107).run(pool).await.unwrap();
    sqlx::raw_sql(include_str!(
        "../../../../../../migrations/0108_run_volume_leases.sql"
    ))
    .execute(pool)
    .await
    .unwrap();
    sqlx::raw_sql(include_str!(
        "../../../../../../migrations/0109_run_cleanup.sql"
    ))
    .execute(pool)
    .await
    .unwrap();
}

pub async fn git(pool: &PgPool) -> StartRun {
    let command = super::seed::normal(pool).await;
    super::support::seed_run_request(pool, &command).await;
    command
}

pub async fn stored(pool: &PgPool, command: &StartRun) -> (String, bool, bool) {
    sqlx::query_as("SELECT execution_profile,vm_plan_admission_required,num_nonnulls(vm_provider_namespace,vm_host_id,vm_binding_kind)>0 FROM runs WHERE id=$1")
        .bind(command.run_id.as_uuid()).fetch_one(pool).await.unwrap()
}

pub async fn absent(pool: &PgPool, command: &StartRun) {
    let empty: bool = sqlx::query_scalar("SELECT NOT EXISTS(SELECT 1 FROM runs WHERE id=$1) AND NOT EXISTS(SELECT 1 FROM command_inbox WHERE command_id=$2) AND NOT EXISTS(SELECT 1 FROM run_events WHERE run_id=$1)")
        .bind(command.run_id.as_uuid()).bind(command.command_id.as_uuid()).fetch_one(pool).await.unwrap();
    assert!(empty, "denial must roll back Run, inbox and events");
}
