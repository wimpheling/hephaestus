use authz_postgres::PostgresMelangeAuthorizer;
use release_postgres::ReleaseService;
use run_domain::{FreshRunExecutionProfile, RunState};
use run_orchestrator::RunRepository;
use run_postgres::PgRunRepository;
use sqlx::PgPool;
use std::sync::Arc;
use vm_trait::VmExit;

use super::{
    fixture,
    public_fixture::{Role, role_pool},
    seed,
};

#[sqlx::test(migrations = false)]
#[ignore = "requires actual disposable private lifecycle PostgreSQL"]
async fn qualified_legacy_update_hook_binds_candidate_and_completes_without_owned_receipt(
    pool: PgPool,
) {
    fixture::schema(&pool).await;
    let (identity, hook) = seed::update(&pool).await;
    let worker = role_pool(&pool, Role::Worker).await;
    ReleaseService::new(worker.clone(), Arc::new(PostgresMelangeAuthorizer))
        .with_execution_profile(FreshRunExecutionProfile::LegacyScalar)
        .begin_update_hook(&identity, hook.clone())
        .await
        .unwrap();
    let command = seed::load_hook(&pool, hook.hook_run_id).await;
    let runs = PgRunRepository::new(worker.clone())
        .with_execution_profile(FreshRunExecutionProfile::LegacyScalar);
    assert!(runs.create_run(&command).await.unwrap().created);
    runs.transition(command.run_id, RunState::LeasingVolume, None, None)
        .await
        .unwrap();
    runs.bind_resources(
        command.run_id,
        None,
        None,
        None,
        &command.run_id.to_string(),
    )
    .await
    .unwrap();
    for state in [
        RunState::Provisioning,
        RunState::Starting,
        RunState::Running,
    ] {
        runs.transition(command.run_id, state, None, None)
            .await
            .unwrap();
    }
    runs.transition(
        command.run_id,
        RunState::Succeeded,
        Some(&VmExit {
            code: Some(0),
            signal: None,
        }),
        None,
    )
    .await
    .unwrap();
    runs.transition(command.run_id, RunState::CleaningUp, None, None)
        .await
        .unwrap();
    runs.transition(command.run_id, RunState::CleanedUp, None, None)
        .await
        .unwrap();
    let exact:bool=sqlx::query_scalar("SELECT execution.instance_revision_id=update.candidate_revision_id AND execution.volume_acquisition_closed_at IS NOT NULL AND NOT EXISTS(SELECT 1 FROM run_cleanup_receipts WHERE run_id=execution.id) FROM runs execution JOIN agent_updates update ON update.hook_run_id=execution.id WHERE execution.id=$1")
        .bind(command.run_id.as_uuid()).fetch_one(&pool).await.unwrap();
    assert!(
        exact,
        "candidate pins and scalar closure survive without canonical evidence"
    );
    worker.close().await;
}
