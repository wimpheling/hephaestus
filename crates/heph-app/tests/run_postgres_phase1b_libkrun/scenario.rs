use super::db::{
    cleanup_runtime_records, log_contains, sqlite_previous, stage_update_result, wait_for_state,
};
use super::fixture;
use super::runtime::{command, update_command};
use super::*;

#[tokio::test(flavor = "multi_thread")]
// The sequential end-to-end scenario intentionally keeps all external
// resources alive through both runs so persistence and cleanup are observable.
// Keep this one ordered acceptance flow together so its persistence assertions
// remain adjacent to the VM lifecycle they verify.
#[allow(clippy::too_many_lines)]
async fn real_state_runs_and_update_outcomes_are_isolated_and_reconciled() {
    if env::var(ENABLE_FLAG).as_deref() != Ok("1") {
        return;
    }

    let (pool, repository, runtime_root, cgroup_root, scenario, orchestrator, instance_id) =
        fixture::setup().await;

    let first = command(scenario.current);
    let first_run = orchestrator
        .start_run(&first)
        .await
        .expect("first persistent run");
    assert_eq!(first_run.state, RunState::CleanedUp);
    assert_eq!(sqlite_previous(&pool, first.run_id).await, 0);
    let volume_id = first_run.volume_id.expect("first run volume");
    let backing_path: String =
        sqlx::query_scalar("SELECT host_path FROM agent_instance_state_volumes WHERE id = $1")
            .bind(volume_id.as_uuid())
            .fetch_one(&pool)
            .await
            .expect("volume backing path");
    assert!(PathBuf::from(&backing_path).is_file());

    let second = command(scenario.current);
    let running_orchestrator = Arc::clone(&orchestrator);
    let second_for_task = second.clone();
    let running =
        tokio::spawn(async move { running_orchestrator.start_run(&second_for_task).await });
    wait_for_state(&repository, second.run_id, RunState::Running).await;
    let concurrent = command(scenario.current);
    let rejected = orchestrator
        .start_run(&concurrent)
        .await
        .expect("durably reject concurrent run");
    assert_eq!(rejected.state, RunState::CleanedUp);
    assert_eq!(rejected.outcome, Some(RunOutcome::Failed));
    assert!(
        rejected
            .failure
            .as_deref()
            .is_some_and(|failure| failure.contains("leased by run"))
    );
    let second_run = running
        .await
        .expect("join second run")
        .expect("second persistent run");
    assert_eq!(second_run.state, RunState::CleanedUp);
    assert_eq!(sqlite_previous(&pool, second.run_id).await, 1);
    let active_leases: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM agent_instance_volume_leases WHERE volume_id = $1 AND released_at IS NULL",
    )
    .bind(volume_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("active lease count");
    assert_eq!(active_leases, 0);
    let successful_hook = update_command(scenario.successful);
    let successful_run = orchestrator
        .start_run(&successful_hook)
        .await
        .expect("successful real update hook");
    assert_eq!(successful_run.outcome, Some(RunOutcome::Succeeded));
    assert_eq!(sqlite_previous(&pool, successful_hook.run_id).await, 2);
    let successful_update = AgentUpdateId::new();
    stage_update_result(
        &pool,
        successful_update,
        scenario.current,
        scenario.successful,
        successful_hook.run_id,
    )
    .await;
    let releases = ReleaseService::new(pool.clone(), Arc::new(PostgresMelangeAuthorizer));
    assert_eq!(
        releases
            .reconcile_update_run(successful_hook.run_id)
            .await
            .expect("activate successful real hook"),
        UpdateDecision::Activated
    );
    let post_update = command(scenario.successful);
    let post_update_run = orchestrator
        .start_run(&post_update)
        .await
        .expect("normal run from activated candidate");
    assert_eq!(sqlite_previous(&pool, post_update.run_id).await, 3);
    assert!(
        log_contains(
            &pool,
            post_update.run_id,
            &format!("release_marker={}", scenario.successful.release)
        )
        .await,
        "post-update run must execute the candidate release contract"
    );

    let rejected_hook = update_command(scenario.rejected);
    let rejected_run = orchestrator
        .start_run(&rejected_hook)
        .await
        .expect("explicitly rejected real update hook");
    assert_eq!(rejected_run.outcome, Some(RunOutcome::Failed));
    assert_eq!(
        rejected_run.exit.as_ref().and_then(|exit| exit.code),
        Some(23)
    );
    let rejected_update = AgentUpdateId::new();
    stage_update_result(
        &pool,
        rejected_update,
        scenario.successful,
        scenario.rejected,
        rejected_hook.run_id,
    )
    .await;
    assert_eq!(
        releases
            .reconcile_update_run(rejected_hook.run_id)
            .await
            .expect("preserve current revision after agent rollback"),
        UpdateDecision::AgentRejected
    );
    let after_rejection = command(scenario.successful);
    let after_rejection_run = orchestrator
        .start_run(&after_rejection)
        .await
        .expect("normal run after explicit rollback");
    assert_eq!(
        sqlite_previous(&pool, after_rejection.run_id).await,
        4,
        "the rejected hook must not retain its transactional mutation"
    );

    let uncertain_hook = update_command(scenario.uncertain);
    let uncertain_run = orchestrator
        .start_run(&uncertain_hook)
        .await
        .expect("forced update termination is durably cleaned");
    assert_eq!(uncertain_run.outcome, Some(RunOutcome::Failed));
    assert!(
        uncertain_run
            .failure
            .as_deref()
            .is_some_and(|failure| failure.contains("wall-clock timeout"))
    );
    let uncertain_update = AgentUpdateId::new();
    stage_update_result(
        &pool,
        uncertain_update,
        scenario.successful,
        scenario.uncertain,
        uncertain_hook.run_id,
    )
    .await;
    assert_eq!(
        releases
            .reconcile_update_run(uncertain_hook.run_id)
            .await
            .expect("pause after forced update termination"),
        UpdateDecision::CompatibilityUnknown
    );
    let paused: (uuid::Uuid, String, bool) = sqlx::query_as(
        "SELECT active_revision_id, state, run_gate_open
         FROM agent_instances WHERE id = $1",
    )
    .bind(instance_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("paused uncertain instance");
    assert_eq!(
        paused,
        (
            scenario.successful.revision.as_uuid(),
            String::from("paused_unknown_state"),
            false,
        )
    );

    for run in [
        &first_run,
        &second_run,
        &successful_run,
        &post_update_run,
        &rejected_run,
        &after_rejection_run,
        &uncertain_run,
    ] {
        let vm_id = run.vm_id.as_deref().expect("run VM ID");
        assert!(!runtime_root.join(vm_id).exists());
        assert!(!cgroup_root.join(vm_id).exists());
    }
    assert!(PathBuf::from(&backing_path).is_file());
    let active_leases_after_updates: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM agent_instance_volume_leases
         WHERE volume_id = $1 AND released_at IS NULL",
    )
    .bind(volume_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("active lease count after updates");
    assert_eq!(active_leases_after_updates, 0);
    let retained_releases: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM releases
         WHERE id = ANY($1)",
    )
    .bind(vec![
        scenario.current.release.as_uuid(),
        scenario.successful.release.as_uuid(),
        scenario.rejected.release.as_uuid(),
        scenario.uncertain.release.as_uuid(),
    ])
    .fetch_one(&pool)
    .await
    .expect("retained update releases");
    assert_eq!(retained_releases, 4);

    cleanup_runtime_records(&pool, instance_id).await;
}
