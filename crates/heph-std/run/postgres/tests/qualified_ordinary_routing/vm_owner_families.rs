//! Ordinary public139 producer ledgers, with no VM start or physical-stop claim.

#[path = "vm_owner_families/update.rs"]
mod update;

use release_service::UpdateHookResult;
use run_domain::{FreshRunExecutionProfile, VmOwnerHistory, VmOwnerHistoryConflict};
use run_postgres::PgVmOwnerHistoryReader;
use sqlx::PgPool;
use uuid::Uuid;
use vm_trait::VmProviderOwnerScope;

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable public139 PostgreSQL through DATABASE_URL and local Git"]
async fn canonical_update_consumed_then_cancelled_original_survives_uncertain_producer(
    pool: PgPool,
) {
    let f = update::Fixture::new(&pool, FreshRunExecutionProfile::OwnedCanonical).await;
    let reader = PgVmOwnerHistoryReader::new(f.original.worker.clone());
    let owner = f.original.scope.provider_scope();
    assert!(matches!(
        reader.observe(owner).await.unwrap(),
        VmOwnerHistory::Unknown(_)
    ));
    // The actor-produced queued Update has no original VM identity until this
    // real worker operation consumes the exact protected planning admission.
    f.consume().await;
    assert_eq!(reader.observe(owner).await.unwrap(), VmOwnerHistory::Exact);
    f.cancel().await;
    assert_eq!(reader.observe(owner).await.unwrap(), VmOwnerHistory::Exact);
    // This is the production worker's uncertainty ledger callback, not an exit,
    // drain or physical receipt. No VM was started or declared stopped here.
    f.service
        .record_update_hook_result(f.update, UpdateHookResult::Uncertain)
        .await
        .unwrap();
    f.assert_uncertain(&pool).await;
    assert_eq!(reader.observe(owner).await.unwrap(), VmOwnerHistory::Exact);
    let other_namespace =
        VmProviderOwnerScope::new(Uuid::new_v4().to_string(), owner.host_id().into()).unwrap();
    assert_eq!(
        reader.observe(&other_namespace).await.unwrap(),
        VmOwnerHistory::Conflict(VmOwnerHistoryConflict::NamespaceMismatch)
    );
    f.assert_original_command_cannot_disappear().await;
    assert_eq!(reader.observe(owner).await.unwrap(), VmOwnerHistory::Exact);
    f.original.close().await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable public139 PostgreSQL through DATABASE_URL and local Git"]
async fn strict_legacy_update_closed_terminal_original_survives_uncertain_producer(pool: PgPool) {
    let f = update::Fixture::new(&pool, FreshRunExecutionProfile::LegacyScalar).await;
    let reader = PgVmOwnerHistoryReader::new(f.original.worker.clone());
    let owner = f.original.scope.provider_scope();
    // A real strict Legacy birth already pins owner, even before consumption.
    assert_eq!(reader.observe(owner).await.unwrap(), VmOwnerHistory::Exact);
    f.consume().await;
    f.cancel().await;
    update::finish_pre_io(&f.runs, &f.start, &f.original.scope).await;
    assert_eq!(reader.observe(owner).await.unwrap(), VmOwnerHistory::Exact);
    f.service
        .record_update_hook_result(f.update, UpdateHookResult::Uncertain)
        .await
        .unwrap();
    f.assert_uncertain(&pool).await;
    assert_eq!(reader.observe(owner).await.unwrap(), VmOwnerHistory::Exact);
    let other_namespace =
        VmProviderOwnerScope::new(Uuid::new_v4().to_string(), owner.host_id().into()).unwrap();
    assert_eq!(
        reader.observe(&other_namespace).await.unwrap(),
        VmOwnerHistory::Conflict(VmOwnerHistoryConflict::NamespaceMismatch)
    );
    f.assert_original_command_cannot_disappear().await;
    f.original.close().await;
}
