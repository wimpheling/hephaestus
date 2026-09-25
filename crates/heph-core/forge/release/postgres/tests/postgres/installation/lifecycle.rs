use super::*;

#[path = "lifecycle/replay_matrix.rs"]
mod replay_matrix;
#[path = "lifecycle/scope_matrix.rs"]
mod scope_matrix;

struct LifecycleContext {
    admin_pool: PgPool,
    worker_pool: PgPool,
    service: ReleaseService,
    fixture: Fixture,
    release_v1: ReleaseId,
    installation: InstallStaticUiResult,
    activation: UiInstallationGenerationResult,
}

#[tokio::test]
#[serial]
// Every activation and rollback pins a fresh generation. This matrix also
// checks receipt replay after source revocation and rollback rollback safety.
async fn ui_installation_activation_rollback_generation_matrix() {
    let Some(admin_pool) = pool().await else {
        return;
    };
    sqlx::migrate!("../../../../../migrations")
        .run(&admin_pool)
        .await
        .expect("apply application migrations");
    let Some(worker_pool) = worker_pool_named("heph-ui-generation-matrix").await else {
        return;
    };
    let context = scope_matrix::prepare(admin_pool, worker_pool).await;
    replay_matrix::verify(context).await;
}
