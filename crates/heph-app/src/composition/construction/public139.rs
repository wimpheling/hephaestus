//! Actual ordinary composition on public143, with an in-memory VM backend.
//!
//! `PostgreSQL`, Git, imported artifacts, authority, secrets and local runtime are
//! real. These tests do not certify a native VM, physical drain or recovery.

#[path = "public139/config.rs"]
mod config;
#[path = "public139/git.rs"]
mod git;
#[path = "public139/mailbox.rs"]
mod mailbox;
#[path = "public139/parents.rs"]
mod parents;
#[path = "public139/setup.rs"]
mod setup;

use heph_run::{CancelRun, RunOutcome, RunState};
use runtime_types::CommandId;
use sqlx::PgPool;
use std::{sync::Arc, time::Duration};

use setup::Fixture;

// Consuming Fixture::close(self).await retains roots and pools through shutdown;
// Rust 1.88's suggested drop after that move is invalid.
#[allow(clippy::significant_drop_tightening)]
#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL143, loopback NATS and local Git"]
async fn ordinary_public143_git_and_mailbox_use_actual_legacy_assembly(pool: PgPool) {
    let fixture = Box::pin(Fixture::new(&pool)).await;
    let start = fixture.accept_git().await;
    execute_and_cancel(&fixture, start).await;

    // Allocation, acceptance, wake and dispatch all use the actual mailbox
    // repository retained by App construction, including its configured profile.
    let start = mailbox::dispatch(&fixture).await;
    execute_and_cancel(&fixture, start.clone()).await;
    mailbox::assert_settled_cancel(&pool, start.run_id).await;
    fixture.close().await;
}

async fn execute_and_cancel(fixture: &Fixture, start: heph_run::StartRun) {
    let orchestrator = Arc::clone(&fixture.app.orchestrator);
    let command = start.clone();
    let mut execution = tokio::spawn(async move { orchestrator.start_run(&command).await });
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        let state: Option<String> = sqlx::query_scalar("SELECT state FROM runs WHERE id=$1")
            .bind(start.run_id.as_uuid())
            .fetch_optional(&fixture.pool)
            .await
            .expect("observe actual execution");
        if state.as_deref() == Some("running") {
            break;
        }
        assert!(
            !execution.is_finished(),
            "ordinary execution ended before Running: {:?}",
            execution.await
        );
        assert!(
            tokio::time::Instant::now() < deadline,
            "ordinary execution did not reach Running"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(
        fixture
            .app
            .orchestrator
            .cancel_run(&CancelRun {
                command_id: CommandId::new(),
                run_id: start.run_id,
                reason: "ordinary public141 assembly cancellation".into(),
            })
            .await
            .expect("actual cancellation")
    );
    let run = tokio::time::timeout(Duration::from_secs(30), &mut execution)
        .await
        .expect("bounded actual cleanup")
        .expect("execution task")
        .expect("ordinary lifecycle and concrete completion observer");
    assert_eq!(run.state, RunState::CleanedUp);
    assert_eq!(run.outcome, Some(RunOutcome::Cancelled));
    let exact: bool = sqlx::query_scalar(
        "SELECT execution_profile='legacy_scalar' AND NOT vm_plan_admission_required
            AND vm_provider_namespace IS NULL AND vm_host_id IS NULL
            AND vm_binding_kind IS NULL AND vm_id=id::text
            AND volume_id IS NULL AND lease_id IS NULL
            AND volume_acquisition_closed_at IS NOT NULL
            AND cancel_requested_at IS NOT NULL AND state='cleaned_up'
            AND NOT EXISTS(SELECT 1 FROM run_legacy_vm_placements WHERE run_id=$1)
            AND EXISTS(SELECT 1 FROM runtime_authority_sessions
                WHERE run_id=$1 AND status='revoked' AND acknowledged_at IS NOT NULL)
            AND EXISTS(SELECT 1 FROM run_workspaces
                WHERE run_id=$1 AND state='cleaned' AND cleaned_at IS NOT NULL)
         FROM runs WHERE id=$1",
    )
    .bind(start.run_id.as_uuid())
    .fetch_one(&fixture.pool)
    .await
    .expect("exact scalar terminal evidence");
    assert!(exact);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL with ordinary public138"]
async fn ordinary_public138_is_rejected_before_filesystem_initialization(pool: PgPool) {
    setup::install(&pool, 138).await;
    let root = tempfile::tempdir().expect("isolated application roots");
    let config = config::app_config(
        setup::database_url(&pool),
        "nats://127.0.0.1:1".into(),
        root.path(),
    );
    let paths = [
        config.repository_root.clone(),
        config.volumes.volume_root.clone(),
        config.workspaces.workspace_root.clone(),
        config.run_runtime.runtime_root.clone(),
    ];
    assert!(config.managed_backend.is_none());
    let error = match super::build(config, None).await {
        Ok(_) => panic!("public138 must not construct public143 application"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("expected 143"));
    for path in paths {
        assert_eq!(
            std::fs::read_dir(path)
                .expect("unchanged fixture root")
                .count(),
            0
        );
    }
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL107 to143, loopback NATS and local Git"]
async fn ordinary_public143_does_not_promote_genuine_public107_original(pool: PgPool) {
    let (fixture, start) = Box::pin(Fixture::historical(&pool)).await;
    let before: serde_json::Value =
        sqlx::query_scalar("SELECT to_jsonb(execution) FROM runs execution WHERE id=$1")
            .bind(start.run_id.as_uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(before["execution_profile"], "historical_unproven");
    assert_eq!(before["state"], "queued");
    let error = fixture
        .app
        .orchestrator
        .start_run(&start)
        .await
        .expect_err("original history must remain held");
    assert!(error.to_string().contains("replay cannot promote history"));
    let after: serde_json::Value =
        sqlx::query_scalar("SELECT to_jsonb(execution) FROM runs execution WHERE id=$1")
            .bind(start.run_id.as_uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(after, before);
    let no_session: bool = sqlx::query_scalar(
        "SELECT NOT EXISTS(SELECT 1 FROM runtime_authority_sessions WHERE run_id=$1)",
    )
    .bind(start.run_id.as_uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(no_session);
    fixture.close().await;
}
