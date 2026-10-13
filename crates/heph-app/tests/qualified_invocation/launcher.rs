use heph_run::{RunAuthorityManager, RunKind};
use hephaestus_app::{CanonicalRunLaunchAuthorizer, PgRunAuthorityManager};
use run_orchestrator::RunLaunchAuthorizer;
use runtime_authority_postgres::PgRuntimeSessionRepository;
use runtime_git_authority_postgres::PgRuntimeGitCredentialRepository;
use sqlx::PgPool;
use std::{sync::Arc, time::Duration};

use super::fixture::Fixture;

async fn launch_phases(pool: &PgPool, count: usize) {
    let f = Box::pin(Fixture::leasing(pool, count)).await;
    let run = f.run().await;
    let before = f.rows().await;
    let launcher = f.launcher();
    authorize_leasing_without_issuance(&f, &run, &launcher, &before).await;
    // The supplied DTO claims Provisioning while the genuine stored run remains
    // Leasing. This cannot bypass either launch entry point, including empty sets.
    let mut supplied = run.clone();
    supplied.state = heph_run::RunState::Provisioning;
    f.denied_unchanged(&supplied).await;
    f.hold_for(time::Duration::hours(2)).await;
    let acquired = f.run().await;
    assert_eq!(acquired.state, heph_run::RunState::LeasingVolume);
    f.denied_unchanged(&acquired).await;
    f.provision().await;
    let current = f.run().await;
    let before = f.rows().await;
    launcher.authorize(&current).await.unwrap();
    launcher.authorize_with_volumes(&current).await.unwrap();
    f.manager().reauthorize(&current).await.unwrap();
    assert_eq!(f.rows().await, before);
    assert!(f.handoff_files().is_empty());
    f.birth.physical.assert_unchanged();
}
async fn authorize_leasing_without_issuance(
    f: &Fixture,
    run: &heph_run::Run,
    launcher: &CanonicalRunLaunchAuthorizer,
    before: &serde_json::Value,
) {
    launcher.authorize(run).await.unwrap();
    launcher.authorize_with_volumes(run).await.unwrap();
    f.manager().reauthorize(run).await.unwrap();
    assert!(f.manager().prepare(run).await.is_err());
    assert!(
        f.manager()
            .acknowledge(run, run.id.as_uuid(), 1)
            .await
            .is_err()
    );
    assert_eq!(
        f.rows().await,
        *before,
        "eligibility cannot issue any authority rows"
    );
    assert!(f.handoff_files().is_empty());
}

#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private119 PostgreSQL/native Local prerequisites"]
async fn launcher_zero_set_uses_persisted_leasing_eligibility_then_full_provisioning(pool: PgPool) {
    launch_phases(&pool, 0).await;
}
#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private119 PostgreSQL/native Local prerequisites"]
async fn launcher_two_set_uses_persisted_leasing_eligibility_then_full_provisioning(pool: PgPool) {
    launch_phases(&pool, 2).await;
}
#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private119 PostgreSQL/native Local prerequisites"]
async fn unconfigured_app_routes_reject_actual_empty_invocation_without_scalar_fallback(
    pool: PgPool,
) {
    let f = Box::pin(Fixture::provisioning(&pool, 0)).await;
    let run = f.run().await;
    assert_eq!(run.kind, RunKind::Invocation);
    let before = f.rows().await;
    let worker = f.birth.birth.worker.clone();
    let legacy = PgRunAuthorityManager::new(
        worker.clone(),
        PgRuntimeGitCredentialRepository::new(worker.clone()),
        f.handoff.path().into(),
        [73; 32],
        Duration::from_secs(60),
    )
    .unwrap();
    assert!(legacy.prepare(&run).await.is_err());
    assert!(legacy.reauthorize(&run).await.is_err());
    assert!(legacy.acknowledge(&run, run.id.as_uuid(), 1).await.is_err());
    let scalar = Arc::new(control_plane_postgres::launch::PgRunLaunchAuthorizer::new(
        worker.clone(),
        Arc::new(authz_postgres::PostgresMelangeAuthorizer),
    ));
    let legacy = CanonicalRunLaunchAuthorizer::new(
        PgRuntimeSessionRepository::new(worker).with_canonical_live_snapshots(),
        scalar,
    );
    assert!(legacy.authorize(&run).await.is_err());
    assert!(legacy.authorize_with_volumes(&run).await.is_err());
    assert_eq!(f.rows().await, before);
    assert!(f.handoff_files().is_empty());
}
