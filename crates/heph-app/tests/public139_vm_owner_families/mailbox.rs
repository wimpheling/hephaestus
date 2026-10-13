//! Current manager allocation/control plus genuine trusted external dispatch.
//! This does not assert identity-based publish or gateway runtime session authority.

use run_domain::{
    CancelRun, LegacyVmPlacementConsumption, LegacyVmPlacementProducer, LegacyVmPlacementScope,
    RunState, VmOwnerHistory, VmOwnerHistoryConflict,
};
use run_orchestrator::RunRepository;
use run_postgres::PgVmOwnerHistoryReader;
use runtime_types::CommandId;
use sqlx::PgPool;
use uuid::Uuid;
use vm_trait::VmProviderOwnerScope;

#[path = "mailbox/fixture.rs"]
mod fixture;

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable ordinary public139 PostgreSQL through DATABASE_URL and local Git"]
async fn strict_legacy_mailbox_leased_consumed_cancelled_original_keeps_owner(pool: PgPool) {
    let f = fixture::Fixture::new(&pool).await;
    let owner = f.original.scope.provider_scope();
    let reader = PgVmOwnerHistoryReader::new(f.original.worker.clone());
    let queued = f.original.runs.get(f.start.run_id).await.unwrap();
    assert_eq!(queued.state, RunState::Queued);
    assert!(queued.vm_id.is_none());
    let placement = f
        .original
        .runs
        .legacy_vm_placement(f.start.run_id, &f.original.scope)
        .await
        .unwrap();
    assert_eq!(placement.producer(), LegacyVmPlacementProducer::Mailbox);
    assert!(placement.consumption().is_none());
    assert_eq!(reader.observe(owner).await.unwrap(), VmOwnerHistory::Exact);
    let leased: (String, String, String, String) = sqlx::query_as("SELECT delivery.disposition,attempt.state,attempt.state_access_outcome,attempt.target_commit FROM mailbox_deliveries delivery JOIN mailbox_delivery_attempts attempt ON attempt.event_id=delivery.event_id WHERE attempt.run_id=$1")
        .bind(f.start.run_id.as_uuid()).fetch_one(&pool).await.unwrap();
    assert_eq!(
        leased,
        (
            "leased".into(),
            "leased".into(),
            "no_state".into(),
            f.original.commit.clone()
        )
    );
    let unchanged = original_graph(&pool, &f).await;
    let other =
        VmProviderOwnerScope::new(Uuid::new_v4().to_string(), owner.host_id().into()).unwrap();
    assert_eq!(
        reader.observe(&other).await.unwrap(),
        VmOwnerHistory::Conflict(VmOwnerHistoryConflict::NamespaceMismatch)
    );
    assert!(
        f.original
            .runs
            .create_run_with_legacy_placement(
                &f.start,
                &LegacyVmPlacementScope::new(other).unwrap()
            )
            .await
            .is_err()
    );
    assert_eq!(unchanged, original_graph(&pool, &f).await);
    assert!(
        f.original
            .runs
            .create_run_with_legacy_placement(&f.start, &f.original.scope)
            .await
            .unwrap()
            .created
    );
    let consumed = f
        .original
        .runs
        .legacy_vm_placement(f.start.run_id, &f.original.scope)
        .await
        .unwrap();
    assert_eq!(
        consumed.consumption(),
        Some(LegacyVmPlacementConsumption::Provision)
    );
    let immutable = original_graph(&pool, &f).await;
    assert_eq!(reader.observe(owner).await.unwrap(), VmOwnerHistory::Exact);
    // The actual operator changes the logical leased delivery to terminal Cancelled.
    f.cancel_delivery(&pool).await;
    assert_eq!(reader.observe(owner).await.unwrap(), VmOwnerHistory::Exact);
    cancel_run(&f).await;
    assert_eq!(reader.observe(owner).await.unwrap(), VmOwnerHistory::Exact);
    assert_eq!(immutable, original_graph(&pool, &f).await);
    deny_original_mutation(&f).await;
    assert_eq!(immutable, original_graph(&pool, &f).await);
    assert_eq!(reader.observe(owner).await.unwrap(), VmOwnerHistory::Exact);
    // Terminal Run cancellation is not cleanup or release of a physical resource.
    let unstarted: bool = sqlx::query_scalar("SELECT vm_id IS NULL AND volume_id IS NULL AND lease_id IS NULL AND lease_fencing_token IS NULL AND NOT EXISTS(SELECT 1 FROM run_cleanup_targets WHERE run_id=$1) AND NOT EXISTS(SELECT 1 FROM run_authorization_snapshots WHERE run_id=$1) FROM runs WHERE id=$1")
        .bind(f.start.run_id.as_uuid()).fetch_one(&pool).await.unwrap();
    assert!(unstarted);
    f.original.close().await;
}

async fn original_graph(pool: &PgPool, f: &fixture::Fixture) -> serde_json::Value {
    // Complete records: neither original IDs nor owner/source pins are normalized.
    sqlx::query_scalar("SELECT jsonb_build_object('placement',to_jsonb(plan),'attempt',to_jsonb(attempt),'event',to_jsonb(event)) FROM run_legacy_vm_placements plan JOIN mailbox_delivery_attempts attempt ON attempt.id=plan.producer_id JOIN mailbox_events event ON event.id=attempt.event_id WHERE plan.run_id=$1 AND plan.event_id=$2 AND plan.producer_kind='mailbox'")
        .bind(f.start.run_id.as_uuid()).bind(f.event.as_uuid()).fetch_one(pool).await.unwrap()
}

async fn cancel_run(f: &fixture::Fixture) {
    // Invoke the worker cancellation APIs separately: operator Cancel above
    // changes delivery eligibility and does not itself stop a physical VM.
    assert!(
        f.original
            .runs
            .request_cancel(&CancelRun {
                command_id: CommandId::new(),
                run_id: f.start.run_id,
                reason: "mailbox owner history before provider IO".into(),
            })
            .await
            .unwrap()
    );
    f.original
        .runs
        .transition(f.start.run_id, RunState::Cancelled, None, None)
        .await
        .unwrap();
    assert_eq!(
        f.original.runs.get(f.start.run_id).await.unwrap().state,
        RunState::Cancelled
    );
}

async fn deny_original_mutation(f: &fixture::Fixture) {
    let error = sqlx::query(
        "UPDATE mailbox_delivery_attempts SET target_ref='refs/heads/foreign' WHERE run_id=$1",
    )
    .bind(f.start.run_id.as_uuid())
    .execute(&f.original.worker)
    .await
    .unwrap_err();
    assert_eq!(
        error.as_database_error().unwrap().code().as_deref(),
        Some("23000")
    );
}
