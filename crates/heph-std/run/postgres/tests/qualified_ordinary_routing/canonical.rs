use run_domain::{FreshRunExecutionProfile, RunCleanupHostId};
use run_orchestrator::RunRepository;
use run_postgres::{
    PgRunRepository, QualifiedOrdinaryRoutingDecision as Decision,
    QualifiedOrdinaryRoutingHoldReason as Hold,
};
use sqlx::PgPool;
use vm_trait::VmId;

use super::fixture::Fixture;

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL through DATABASE_URL and local Git"]
async fn genuine_canonical_git_original_is_held_until_worker_facade_exists(pool: PgPool) {
    let f = Fixture::new(&pool).await;
    let owner = f.scope.provider_scope();
    let scope = RunCleanupHostId::new(owner.namespace().into(), owner.host_id().into())
        .expect("actual configured owner metadata");
    let runs = PgRunRepository::new(f.worker.clone())
        .with_execution_profile(FreshRunExecutionProfile::OwnedCanonical)
        .with_vm_owner_scope(scope.clone());
    assert!(
        runs.create_run_with_vm_plan(&f.start, &scope, &VmId(f.start.run_id.to_string()))
            .await
            .expect("real protected canonical Git birth")
            .created
    );
    assert_eq!(
        f.reader.inspect_ordinary_start(&f.start).await.unwrap(),
        Decision::Held(Hold::CanonicalOriginalFacadeRequired)
    );
    assert_eq!(
        f.reader
            .inspect_ordinary_original(f.start.run_id)
            .await
            .unwrap(),
        Decision::Held(Hold::CanonicalOriginalFacadeRequired)
    );
    f.close().await;
}
