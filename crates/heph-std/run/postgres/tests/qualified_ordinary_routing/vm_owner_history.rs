//! Actual Git producers and protected births; observations grant no root authority.

use run_domain::{
    CancelRun, FreshRunExecutionProfile, RunCleanupHostId, RunState, VmOwnerHistory,
    VmOwnerHistoryConflict, VmOwnerHistoryUnknown,
};
use run_orchestrator::RunRepository;
use run_postgres::{PgRunRepository, PgVmOwnerHistoryReader};
use runtime_types::CommandId;
use sqlx::PgPool;
use uuid::Uuid;
use vm_trait::{VmId, VmProviderOwnerScope};

use super::fixture::Fixture;

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable public139 PostgreSQL through DATABASE_URL and local Git"]
async fn genuine_unborn_producer_is_not_vm_history(pool: PgPool) {
    let f = Fixture::new(&pool).await;
    let history = PgVmOwnerHistoryReader::new(f.worker.clone());
    assert_eq!(
        history.observe(f.scope.provider_scope()).await.unwrap(),
        VmOwnerHistory::NoHistoryForConfiguredHost
    );
    f.birth().await;
    assert_eq!(
        history.observe(f.scope.provider_scope()).await.unwrap(),
        VmOwnerHistory::Exact
    );
    f.close().await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable public139 PostgreSQL through DATABASE_URL and local Git"]
async fn actual_terminal_closed_legacy_original_remains_history(pool: PgPool) {
    let f = Fixture::new(&pool).await;
    f.birth().await;
    let cancel = CancelRun {
        command_id: CommandId::new(),
        run_id: f.start.run_id,
        reason: "immutable owner observation".into(),
    };
    assert!(f.runs.request_cancel(&cancel).await.unwrap());
    f.runs
        .transition(f.start.run_id, RunState::Cancelled, None, None)
        .await
        .unwrap();
    f.runs
        .close_legacy_vm_acquisition(f.start.run_id, &f.scope)
        .await
        .unwrap();
    for state in [RunState::CleaningUp, RunState::CleanedUp] {
        f.runs
            .transition(f.start.run_id, state, None, None)
            .await
            .unwrap();
    }
    sqlx::query("UPDATE run_requests SET dispatch_state='cancelled' WHERE run_id=$1")
        .bind(f.start.run_id.as_uuid())
        .execute(&f.worker)
        .await
        .unwrap();
    assert_eq!(
        PgVmOwnerHistoryReader::new(f.worker.clone())
            .observe(f.scope.provider_scope())
            .await
            .unwrap(),
        VmOwnerHistory::Exact
    );
    f.close().await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable public139 PostgreSQL through DATABASE_URL and local Git"]
async fn immutable_other_namespace_conflicts_and_other_host_is_scoped(pool: PgPool) {
    let f = Fixture::new(&pool).await;
    f.birth().await;
    let history = PgVmOwnerHistoryReader::new(f.worker.clone());
    let changed = VmProviderOwnerScope::new(
        Uuid::new_v4().to_string(),
        f.scope.provider_scope().host_id().into(),
    )
    .unwrap();
    assert_eq!(
        history.observe(&changed).await.unwrap(),
        VmOwnerHistory::Conflict(VmOwnerHistoryConflict::NamespaceMismatch)
    );
    let other_host = VmProviderOwnerScope::new(
        f.scope.provider_scope().namespace().into(),
        "independent-host".into(),
    )
    .unwrap();
    assert_eq!(
        history.observe(&other_host).await.unwrap(),
        VmOwnerHistory::NoHistoryForConfiguredHost
    );
    f.close().await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable public139 PostgreSQL through DATABASE_URL and local Git"]
async fn genuine_unclassified_repository_birth_holds_for_every_host(pool: PgPool) {
    let f = Fixture::new(&pool).await;
    assert!(
        PgRunRepository::new(f.worker.clone())
            .create_run(&f.start)
            .await
            .unwrap()
            .created
    );
    let history = PgVmOwnerHistoryReader::new(f.worker.clone());
    for owner in [
        f.scope.provider_scope().clone(),
        VmProviderOwnerScope::new(Uuid::new_v4().to_string(), "independent-host".into()).unwrap(),
    ] {
        assert_eq!(
            history.observe(&owner).await.unwrap(),
            VmOwnerHistory::Unknown(VmOwnerHistoryUnknown::HistoricalUnproven)
        );
    }
    f.close().await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable public139 PostgreSQL through DATABASE_URL and local Git"]
async fn canonical_original_does_not_require_current_dispatch_or_allow_missing_producer(
    pool: PgPool,
) {
    let f = Fixture::new(&pool).await;
    let owner = f.scope.provider_scope();
    let scope = RunCleanupHostId::new(owner.namespace().into(), owner.host_id().into()).unwrap();
    let runs = PgRunRepository::new(f.worker.clone())
        .with_execution_profile(FreshRunExecutionProfile::OwnedCanonical)
        .with_vm_owner_scope(scope.clone());
    assert!(
        runs.create_run_with_vm_plan(&f.start, &scope, &VmId(f.start.run_id.to_string()))
            .await
            .unwrap()
            .created
    );
    let history = PgVmOwnerHistoryReader::new(f.worker.clone());
    assert_eq!(history.observe(owner).await.unwrap(), VmOwnerHistory::Exact);
    sqlx::query("UPDATE run_requests SET dispatch_state='cancelled' WHERE run_id=$1")
        .bind(f.start.run_id.as_uuid())
        .execute(&f.worker)
        .await
        .unwrap();
    assert_eq!(history.observe(owner).await.unwrap(), VmOwnerHistory::Exact);
    let denied = sqlx::query("DELETE FROM run_requests WHERE run_id=$1")
        .bind(f.start.run_id.as_uuid())
        .execute(&f.worker)
        .await
        .unwrap_err();
    assert_eq!(
        denied.as_database_error().unwrap().code().as_deref(),
        Some("23000")
    );
    assert_eq!(history.observe(owner).await.unwrap(), VmOwnerHistory::Exact);
    f.close().await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable public139 PostgreSQL through DATABASE_URL and local Git"]
async fn admin_and_application_roles_cannot_prove_absence(pool: PgPool) {
    let f = Fixture::new(&pool).await;
    for reader in [
        PgVmOwnerHistoryReader::new(pool.clone()),
        PgVmOwnerHistoryReader::new(f.application.clone()),
    ] {
        assert!(reader.observe(f.scope.provider_scope()).await.is_err());
    }
    f.close().await;
}
