//! Three positive native registrations; denial/race matrix remains pending.
use super::{
    authenticated_stamp, cleanup,
    fixture::{self, Fixture},
    observations,
    running::ControlledGuest,
};
use authz_postgres::PostgresMelangeAuthorizer;
use heph_run::{RunOutcome, RunRepository, RunState};
use release_domain::ReleaseCommandKey;
use release_postgres::PostgresInstanceExecutionService;
use release_service::{InstanceExecutionService, InstanceInvocationId, InvokeInstance};
use run_domain::{RunCleanupReceipt, StartRun};
use run_orchestrator::RunCleanupRepository;
use runtime_types::{CommandId, RunId};
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;
use vm_trait::{VmId, VmInstance};
use volume_trait::{RunVolumeMetadataRepository, RunVolumeStore};

async fn commit_success(f: &Fixture, count: usize) -> RunCleanupReceipt {
    let files = observations::files(f);
    let guest = ControlledGuest::new(f);
    guest.start().await.unwrap();
    let exit = guest.wait().await.unwrap();
    assert_eq!(
        exit,
        vm_trait::VmExit {
            code: Some(0),
            signal: None
        }
    );
    let receipt = cleanup::close_after_exit(f, &guest).await;
    let before = observations::rows(f).await;
    let run = f
        .birth
        .repository()
        .transition(f.birth.start.run_id, RunState::Succeeded, Some(&exit), None)
        .await
        .unwrap();
    assert_eq!(run.state, RunState::Succeeded);
    assert_eq!(run.outcome, Some(RunOutcome::Succeeded));
    assert_eq!(run.exit, Some(exit));
    assert!(run.failure.is_none());
    assert_eq!(
        f.run().await,
        run,
        "actual RunPG transition committed before cleanup"
    );
    let after = observations::rows(f).await;
    observations::success(f, &before, &after, count).await;
    assert_eq!(observations::files(f), files);
    receipt
}
#[sqlx::test(migrations = false)]
#[ignore = "requires exact private28+selected29 PostgreSQL and genuine native Local births; controlled VM ports"]
async fn terminal_zero_success_commits_before_cleanup(pool: PgPool) {
    let f = Box::pin(fixture::running(&pool, 0)).await;
    let receipt = commit_success(&f, 0).await;
    // Fresh authenticated closure while Succeeded must preserve the committed result.
    authenticated_stamp::close_and_replay(&f).await;
    cleanup::cleaning(&f).await;
    cleanup::finish(&f, &receipt).await;
}
#[sqlx::test(migrations = false)]
#[ignore = "requires exact private28+selected29 PostgreSQL and genuine native Local births; controlled VM ports"]
async fn terminal_two_success_commits_before_cleanup(pool: PgPool) {
    let f = Box::pin(fixture::running(&pool, 2)).await;
    let receipt = commit_success(&f, 2).await;
    cleanup::cleaning(&f).await;
    cleanup::finish(&f, &receipt).await;
    newer_generation_survives_replay(&f, &receipt).await;
}
#[sqlx::test(migrations = false)]
#[ignore = "requires exact private28+selected29 PostgreSQL and genuine native Local births; controlled VM ports"]
async fn terminal_later_transaction_cleanup_and_stamp_remain_valid(pool: PgPool) {
    let f = Box::pin(fixture::running(&pool, 2)).await;
    let receipt = commit_success(&f, 2).await;
    cleanup::cleaning(&f).await;
    // A separate fresh actual107 Remove admission exercises old CleaningUp/outcomeSucceeded.
    authenticated_stamp::close_and_replay(&f).await;
    cleanup::finish(&f, &receipt).await;
}
async fn newer_generation_survives_replay(f: &Fixture, receipt: &RunCleanupReceipt) {
    let birth = &f.birth.birth;
    let id = Uuid::new_v4();
    let invocation = InvokeInstance::new(
        ReleaseCommandKey::derive("terminal-next-generation", &[id.as_bytes()]),
        InstanceInvocationId::from_uuid(id).unwrap(),
        RunId::new(),
        CommandId::new(),
        birth.input.instance_id(),
        birth.input.revision_id(),
    )
    .unwrap();
    PostgresInstanceExecutionService::new(
        birth.app.clone(),
        Arc::new(PostgresMelangeAuthorizer),
        f.birth.config.clone(),
    )
    .with_qualified_invocation_admission()
    .invoke_instance(&birth.actor, invocation)
    .await
    .unwrap();
    let start: StartRun = serde_json::from_value(
        sqlx::query_scalar::<_, serde_json::Value>(
            "SELECT payload FROM command_inbox WHERE command_id=$1",
        )
        .bind(invocation.start_command_id().as_uuid())
        .fetch_one(&f.birth.pool)
        .await
        .unwrap(),
    )
    .unwrap();
    let repository = f.birth.repository();
    let vm = VmId(start.run_id.to_string());
    let consumed = repository
        .create_run_with_vm_plan(&start, &f.birth.scope, &vm)
        .await
        .unwrap();
    assert!(!consumed.created);
    repository
        .transition(start.run_id, RunState::LeasingVolume, None, None)
        .await
        .unwrap();
    let selected = f.volumes.load_run_selections(start.run_id).await.unwrap();
    f.volumes.preflight_run(&selected).await.unwrap();
    let now = crate::qualified_invocation::fixture::pg_time();
    let held = f
        .birth
        .volumes()
        .acquire_run(
            &selected,
            f.birth.scope.host_id(),
            now,
            now + time::Duration::hours(2),
        )
        .await
        .unwrap();
    assert_newer_generation_fences(f, receipt, &start, &held, &selected);
    let before = observations::rows(f).await;
    let files = observations::files(f);
    let old = f.run().await;
    assert_eq!(
        cleanup::repository(f)
            .finish_cleanup(receipt)
            .await
            .unwrap(),
        old
    );
    assert_eq!(
        observations::rows(f).await,
        before,
        "old completed receipt cannot release or clear genuinely newer held fences"
    );
    assert_eq!(f.volumes.leases_for_run(start.run_id).await.unwrap(), held);
    assert_eq!(observations::files(f), files);
    f.birth.physical.assert_unchanged();
}
fn assert_newer_generation_fences(
    f: &Fixture,
    receipt: &RunCleanupReceipt,
    start: &StartRun,
    held: &[volume_trait::RunVolumeLease],
    selected: &volume_domain::RunVolumeSelections,
) {
    assert_eq!(held.len(), 2);
    assert_ne!(start.run_id, receipt.target().run_id());
    assert_eq!(held.len(), receipt.target().leases().len());
    assert_eq!(held.len(), selected.selections().len());
    assert_eq!(receipt.target().vm_target().host(), Some(&f.birth.scope));
    assert_eq!(
        f.birth.scope.provider_namespace(),
        f.birth.config.provider_scope().namespace()
    );
    assert_eq!(
        f.birth.scope.host_id(),
        f.birth.config.provider_scope().host_id()
    );
    // Acquired leases follow slot order; cleanup fences follow volume order.
    let mut held_by_volume: std::collections::BTreeMap<_, _> = held
        .iter()
        .map(|lease| {
            (
                (lease.lease().volume_id, lease.lease().host_id.clone()),
                lease,
            )
        })
        .collect();
    assert_eq!(held_by_volume.len(), held.len());
    let newer_ids: std::collections::BTreeSet<_> =
        held.iter().map(|lease| lease.lease().id).collect();
    assert_eq!(newer_ids.len(), held.len());
    let original_ids: std::collections::BTreeSet<_> = receipt
        .target()
        .leases()
        .iter()
        .map(run_domain::RunCleanupLeaseFence::lease_id)
        .collect();
    assert_eq!(original_ids.len(), receipt.target().leases().len());
    assert!(newer_ids.is_disjoint(&original_ids));
    let mut selected_by_volume: std::collections::BTreeMap<_, _> = selected
        .selections()
        .iter()
        .map(|selection| (selection.scope().volume_id(), selection))
        .collect();
    assert_eq!(selected_by_volume.len(), selected.selections().len());
    for original in receipt.target().leases() {
        let lease = held_by_volume
            .remove(&(original.volume_id(), original.host_id().to_owned()))
            .expect("newer lease must match the original resource and host");
        let selection = selected_by_volume
            .remove(&original.volume_id())
            .expect("newer lease must belong to the complete selected set");
        assert_eq!(lease.selection(), Some(selection));
        assert_eq!(selection.identity().run_id(), start.run_id);
        assert_eq!(
            selection.identity().instance_id(),
            receipt.target().instance_id()
        );
        assert_eq!(
            selection.identity().revision_id(),
            receipt.target().revision_id()
        );
        assert_eq!(lease.lease().run_id, start.run_id);
        assert_eq!(lease.lease().volume_id, original.volume_id());
        assert_eq!(lease.lease().host_id, original.host_id());
        assert!(lease.lease().fencing_token > original.fencing_token());
        assert_ne!(lease.lease().id, original.lease_id());
    }
    assert!(held_by_volume.is_empty());
    assert!(selected_by_volume.is_empty());
}
