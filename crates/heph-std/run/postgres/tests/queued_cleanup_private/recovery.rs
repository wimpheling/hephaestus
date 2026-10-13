//! Failed scoped callback retains metadata; later success/reuse needs real proof.
use super::{
    controller::{Control, build},
    observations::{rows, unconsumed},
    queued_fixture::Queued,
};
use run_orchestrator::{
    OrchestratorError, RepositoryError, RunCleanupRepository, RunCleanupWakeProgress, RunRepository,
};
use sqlx::PgPool;
use std::sync::atomic::Ordering;

#[sqlx::test(migrations = false)]
async fn failed_scoped_callback_retains_target_then_real_receipt_replay_preserves_newer_holder(
    pool: PgPool,
) {
    let h = Queued::new(&pool, 2, true).await;
    let prepared = h.prepare().await;
    let target = h.target().await;
    let original_volumes = rows(&pool).await["agent_instance_state_volumes"].clone();
    let control = Control::new();
    control.fail.store(true, Ordering::SeqCst);
    let core = build(&h, control.clone(), None, None);
    assert!(matches!(
        core.clone().wake_cleanup(&h.request).await,
        Err(OrchestratorError::Vm(vm_trait::VmError::InvalidState(
            "controlled scoped failure"
        )))
    ));
    assert_eq!(
        h.repository().get(h.request.run_id()).await.unwrap().state,
        run_domain::RunState::Cancelled
    );
    assert_eq!(h.read().await.binding, prepared.binding);
    assert_eq!(h.target().await, target);
    assert!(
        h.cleanup()
            .recorded_cleanup_receipt(h.request.run_id())
            .await
            .unwrap()
            .is_none()
    );
    let counts:(i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM run_cleanup_completions WHERE run_id=$1),(SELECT count(*) FROM agent_instance_volume_leases WHERE run_id=$1)")
        .bind(h.request.run_id().as_uuid()).fetch_one(&pool).await.unwrap();
    assert_eq!(counts, (0, 0));
    assert_eq!(
        rows(&pool).await["agent_instance_state_volumes"],
        original_volumes
    );
    unconsumed(&h).await;
    h.fixture.physical.assert_unchanged();
    control.fail.store(false, Ordering::SeqCst);
    assert_eq!(
        core.clone().wake_cleanup(&h.request).await.unwrap(),
        RunCleanupWakeProgress::Complete
    );
    let terminal = h.read().await;
    assert_eq!(terminal.binding, prepared.binding);
    let receipt = h
        .cleanup()
        .recorded_cleanup_receipt(h.request.run_id())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(receipt.target(), &target);
    assert_eq!(control.count("scoped-failed"), 1);
    assert_eq!(control.count("scoped-confirmed"), 1);
    assert_eq!(control.count("completion"), 1);
    let newer = super::successor::acquire_newer(&h.fixture).await;
    let newer_before = super::successor::holder(&pool, newer).await;
    let stable = rows(&pool).await;
    denied_receipt_effects(&h, &receipt).await;
    assert_eq!(
        h.cleanup().finish_cleanup(&receipt).await.unwrap(),
        terminal.run
    );
    assert_eq!(
        core.wake_cleanup(&h.request).await.unwrap(),
        RunCleanupWakeProgress::Complete
    );
    assert_eq!(h.read().await, terminal);
    assert_eq!(h.target().await, target);
    assert_eq!(rows(&pool).await, stable);
    assert_eq!(super::successor::holder(&pool, newer).await, newer_before);
    assert_eq!(control.count("scoped-confirmed"), 1);
    unconsumed(&h).await;
    h.fixture.physical.assert_unchanged();
    h.fixture.birth.assert_historical_migrations(&pool).await;
}
async fn denied_receipt_effects(h: &Queued, receipt: &run_domain::RunCleanupReceipt) {
    let before = rows(&h.fixture.pool).await;
    let default = run_postgres::PgRunCleanupRepository::new(
        h.fixture.birth.worker.clone(),
        h.fixture.scope.clone(),
    );
    assert!(matches!(
        default.finish_cleanup(receipt).await,
        Err(RepositoryError::InvalidData(
            "qualified Invocation configuration is absent"
        ))
    ));
    assert_eq!(rows(&h.fixture.pool).await, before);
    for index in 0..7 {
        let cleanup = run_postgres::PgRunCleanupRepository::new(
            h.fixture.birth.worker.clone(),
            h.fixture.scope.clone(),
        )
        .with_qualified_queued_invocation_cleanup(super::pins::configuration(h, index));
        let expected = if index < 2 {
            "Invocation composition differs"
        } else {
            "cleanup wake history is held or contradictory"
        };
        for error in [
            cleanup.record_cleanup_receipt(receipt).await.unwrap_err(),
            cleanup.finish_cleanup(receipt).await.unwrap_err(),
        ] {
            assert!(matches!(error,RepositoryError::InvalidData(message) if message==expected));
            assert_eq!(
                rows(&h.fixture.pool).await,
                before,
                "receipt/finish actual full configuration {index}"
            );
        }
    }
    let target = receipt.target();
    let changed = run_domain::RunCleanupTarget::new(
        target.run_id(),
        target.instance_id(),
        target.revision_id(),
        target.vm_target().clone(),
        target.generation() + 1,
        target.leases().to_vec(),
    )
    .unwrap();
    let wrong = run_domain::RunCleanupReceipt::new(
        changed,
        receipt.observation().clone(),
        receipt.observed_at(),
    )
    .unwrap();
    for error in [
        h.cleanup()
            .record_cleanup_receipt(&wrong)
            .await
            .unwrap_err(),
        h.cleanup().finish_cleanup(&wrong).await.unwrap_err(),
    ] {
        let RepositoryError::Storage(source) = error else {
            panic!("typed target mismatch: {error:?}")
        };
        assert_eq!(
            source.downcast_ref::<run_domain::RunCleanupError>(),
            Some(&run_domain::RunCleanupError::TargetMismatch)
        );
        assert_eq!(rows(&h.fixture.pool).await, before);
    }
}
