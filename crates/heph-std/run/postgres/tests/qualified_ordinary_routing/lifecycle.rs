use run_domain::{CancelRun, RunState};
use run_orchestrator::RunRepository;
use runtime_types::CommandId;
use sqlx::PgPool;

use super::fixture::Fixture;

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL through DATABASE_URL and local Git"]
async fn cancelled_closed_completed_original_ignores_current_dispatch_and_gate(pool: PgPool) {
    let f = Fixture::new(&pool).await;
    f.birth().await;
    let before = f.original().await;
    let cancel = CancelRun {
        command_id: CommandId::new(),
        run_id: f.start.run_id,
        reason: "reader original lifecycle test".into(),
    };
    assert!(
        f.runs
            .request_cancel(&cancel)
            .await
            .expect("real original-run cancellation")
    );
    f.runs
        .transition(f.start.run_id, RunState::Cancelled, None, None)
        .await
        .expect("real cancelled transition");
    f.runs
        .close_legacy_vm_acquisition(f.start.run_id, &f.scope)
        .await
        .expect("real permanent acquisition closure");
    for state in [RunState::CleaningUp, RunState::CleanedUp] {
        f.runs
            .transition(f.start.run_id, state, None, None)
            .await
            .expect("genuine pre-IO zero-resource terminal transition");
    }
    // Mutable disposition and gate changes are deliberately after real completion.
    // They manufacture no positive original, placement, consumption or receipt.
    sqlx::query("UPDATE run_requests SET dispatch_state='cancelled' WHERE run_id=$1")
        .bind(f.start.run_id.as_uuid())
        .execute(&f.worker)
        .await
        .expect("actual mutable producer disposition");
    sqlx::query("UPDATE agent_instances SET run_gate_open=false WHERE id=$1")
        .bind(f.start.instance_id.as_uuid())
        .execute(&pool)
        .await
        .expect("current gate closed");
    let original = f.original().await;
    assert_eq!(original.original_start(), before.original_start());
    assert_eq!(original.source(), before.source());
    assert_eq!(original.profile(), before.profile());
    assert_eq!(original.owner(), before.owner());
    assert_eq!(original.vm_id(), before.vm_id());
    assert_eq!(original.evidence(), before.evidence());
    assert_eq!(original.run_state(), RunState::CleanedUp);
    assert!(original.cancellation_requested());
    assert!(original.acquisition_closed());
    f.close().await;
}
