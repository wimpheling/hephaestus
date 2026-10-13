use std::sync::Arc;

use authz_postgres::PostgresMelangeAuthorizer;
use release_postgres::ReleaseService;
use run_domain::{FreshRunExecutionProfile, LegacyVmPlacementScope, StartRun};
use run_postgres::PgRunRepository;
use sqlx::PgPool;
use vm_trait::VmProviderOwnerScope;

use super::public_fixture::{Role, role_pool};

pub use super::base::command;

pub async fn schema(pool: &PgPool) {
    super::base::schema(pool).await;
    sqlx::raw_sql(include_str!(
        "../../../../../../migrations/0112_legacy_vm_placement.sql"
    ))
    .execute(pool)
    .await
    .unwrap();
}

pub fn scope() -> LegacyVmPlacementScope {
    LegacyVmPlacementScope::new(
        VmProviderOwnerScope::new(uuid::Uuid::new_v4().to_string(), "lifecycle-host".into())
            .unwrap(),
    )
    .unwrap()
}

pub async fn fresh(
    pool: &PgPool,
    state: bool,
    owner: &LegacyVmPlacementScope,
) -> super::base::Fixture {
    let mut f = super::base::Fixture::new(pool, state).await;
    f.runs = f.runs.with_legacy_vm_placement(owner.clone());
    f
}

pub async fn hook(
    pool: &PgPool,
    owner: &LegacyVmPlacementScope,
) -> (StartRun, PgRunRepository, PgPool) {
    let (identity, hook) = super::seed::update(pool).await;
    let worker = role_pool(pool, Role::Worker).await;
    ReleaseService::new(worker.clone(), Arc::new(PostgresMelangeAuthorizer))
        .with_execution_profile(FreshRunExecutionProfile::LegacyScalar)
        .with_legacy_vm_placement(owner.clone())
        .begin_update_hook(&identity, hook.clone())
        .await
        .unwrap();
    let command = super::seed::load_hook(pool, hook.hook_run_id).await;
    let runs = PgRunRepository::new(worker.clone())
        .with_execution_profile(FreshRunExecutionProfile::LegacyScalar)
        .with_legacy_vm_placement(owner.clone());
    (command, runs, worker)
}

pub async fn close(pool: &PgPool, command: &StartRun) {
    sqlx::query("UPDATE runs SET volume_acquisition_closed_at=COALESCE(volume_acquisition_closed_at,now()) WHERE id=$1")
        .bind(command.run_id.as_uuid()).execute(pool).await.unwrap();
}

pub async fn rows(pool: &PgPool, run: runtime_types::RunId) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM runs WHERE id=$1")
        .bind(run.as_uuid())
        .fetch_one(pool)
        .await
        .unwrap()
}
