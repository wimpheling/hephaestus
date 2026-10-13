use std::sync::Arc;

use authz_postgres::PostgresMelangeAuthorizer;
use release_postgres::ReleaseService;
use run_domain::FreshRunExecutionProfile;
use run_orchestrator::RunRepository;
use run_postgres::PgRunRepository;
use sqlx::PgPool;
use vm_trait::VmId;

use super::{
    fixture::{Role, role_pool, scope},
    profile_support, seed,
};

#[sqlx::test(migrations = false)]
#[ignore = "requires actual disposable PostgreSQL"]
async fn default_update_constructor_preserves_public107_without_schema_probe(pool: PgPool) {
    super::fixture::checkpoint(107).run(&pool).await.unwrap();
    let (identity, hook) = seed::update(&pool).await;
    let worker = role_pool(&pool, Role::Worker).await;
    let service = ReleaseService::new(worker.clone(), Arc::new(PostgresMelangeAuthorizer));
    service
        .begin_update_hook(&identity, hook.clone())
        .await
        .unwrap();
    service.begin_update_hook(&identity, hook).await.unwrap();
    worker.close().await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires actual private profile schema"]
async fn default_update_does_not_opt_into_owned_from_table_presence(pool: PgPool) {
    profile_support::profile_schema(&pool).await;
    let (identity, hook) = seed::update(&pool).await;
    let worker = role_pool(&pool, Role::Worker).await;
    ReleaseService::new(worker.clone(), Arc::new(PostgresMelangeAuthorizer))
        .begin_update_hook(&identity, hook.clone())
        .await
        .unwrap();
    let command = seed::load_hook(&pool, hook.hook_run_id).await;
    assert_eq!(
        profile_support::stored(&pool, &command).await,
        ("historical_unproven".into(), false, false)
    );
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM run_vm_plan_admissions WHERE run_id=$1")
            .bind(hook.hook_run_id.as_uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(count, 0);
    worker.close().await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires actual private profile schema"]
async fn legacy_update_needs_no_plan_and_replay_cannot_change_profile(pool: PgPool) {
    profile_support::profile_schema(&pool).await;
    let (identity, hook) = seed::update(&pool).await;
    let worker = role_pool(&pool, Role::Worker).await;
    let legacy = ReleaseService::new(worker.clone(), Arc::new(PostgresMelangeAuthorizer))
        .with_execution_profile(FreshRunExecutionProfile::LegacyScalar);
    legacy
        .begin_update_hook(&identity, hook.clone())
        .await
        .unwrap();
    let command = seed::load_hook(&pool, hook.hook_run_id).await;
    assert_eq!(
        profile_support::stored(&pool, &command).await,
        ("legacy_scalar".into(), false, false)
    );
    legacy
        .begin_update_hook(&identity, hook.clone())
        .await
        .unwrap();
    let owned = ReleaseService::new(worker.clone(), Arc::new(PostgresMelangeAuthorizer))
        .with_execution_profile(FreshRunExecutionProfile::OwnedCanonical);
    assert!(owned.begin_update_hook(&identity, hook).await.is_err());
    assert_eq!(
        profile_support::stored(&pool, &command).await,
        ("legacy_scalar".into(), false, false)
    );
    worker.close().await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires actual private profile schema"]
async fn owned_update_commits_exact_admission_and_worker_consumes_before_io(pool: PgPool) {
    profile_support::profile_schema(&pool).await;
    let (identity, hook) = seed::update(&pool).await;
    let worker = role_pool(&pool, Role::Worker).await;
    let service = ReleaseService::new(worker.clone(), Arc::new(PostgresMelangeAuthorizer))
        .with_execution_profile(FreshRunExecutionProfile::OwnedCanonical);
    service
        .begin_update_hook(&identity, hook.clone())
        .await
        .unwrap();
    let command = seed::load_hook(&pool, hook.hook_run_id).await;
    assert_eq!(
        profile_support::stored(&pool, &command).await,
        ("owned_canonical".into(), true, false)
    );
    let exact: bool = sqlx::query_scalar("SELECT actor_id=$2 AND request_id=$3 AND producer_command_key=$4 AND consumed_at IS NULL FROM run_vm_plan_admissions WHERE run_id=$1")
        .bind(hook.hook_run_id.as_uuid()).bind(identity.user_id.as_uuid()).bind(identity.request_id.as_uuid())
        .bind(hook.command_key.as_bytes().as_slice()).fetch_one(&pool).await.unwrap();
    assert!(exact);
    assert!(
        sqlx::query("UPDATE runs SET state='leasing_volume' WHERE id=$1")
            .bind(command.run_id.as_uuid())
            .execute(&worker)
            .await
            .is_err()
    );
    let repository = PgRunRepository::new(worker.clone())
        .with_execution_profile(FreshRunExecutionProfile::OwnedCanonical)
        .with_vm_owner_scope(scope());
    repository
        .create_run_with_vm_plan(&command, &scope(), &VmId(command.run_id.to_string()))
        .await
        .unwrap();
    let consumed: bool = sqlx::query_scalar("SELECT consumption_kind='provision' AND consumed_at IS NOT NULL FROM run_vm_plan_admissions WHERE run_id=$1")
        .bind(command.run_id.as_uuid()).fetch_one(&pool).await.unwrap();
    assert!(consumed);
    assert_eq!(
        profile_support::stored(&pool, &command).await,
        ("owned_canonical".into(), true, true)
    );
    worker.close().await;
}
