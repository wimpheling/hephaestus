use super::*;

#[allow(clippy::needless_borrow, clippy::too_many_lines)]
pub async fn run_cooking_browser_phase(
    prep: &mut CookingPreparation,
    update_sequence: Option<&cooking_updates::UpdateSequence>,
) {
    let pool = &prep.pool;
    let restarted = prep
        .running
        .as_ref()
        .expect("running daemon for browser phase");
    let database_url = &prep.database_url;
    let project = &prep.project;
    let organization_id = prep.organization_id;
    let instance = prep.instance;
    let actual_instance = &prep.actual_instance;
    let actual_fixture = &mut prep.actual_fixture;
    let builds = &prep.builds;
    let update_builds = prep.update_builds.as_ref();
    let cooking_update_rule_ids = prep.cooking_update_rule_ids;
    let retry_fixture = prep.retry_fixture.as_ref();
    let workload_phase_timing = prep.workload_phase_timing;
    let browser_abnormal_release_agent_id =
        update_builds.map(|builds| builds.abnormal.release_agent_id);
    let browser_e2e = prep.browser_e2e;
    assert!(browser_e2e, "browser phase called without browser E2E");

    let (completed_run_id, result_commit, result_ref, target_ref): (
        uuid::Uuid,
        String,
        String,
        String,
    ) = sqlx::query_as(
        "SELECT run.id, result.result_commit, result.result_ref,
                    proposal.target_ref
               FROM runs run
               JOIN run_results result ON result.run_id = run.id
               JOIN review_proposals proposal ON proposal.run_id = run.id
              WHERE run.instance_id = $1 AND result.state = 'completed'
                AND proposal.state = 'approved'
                AND result.result_commit IS NOT NULL
              ORDER BY result.completed_at DESC, result.id DESC
              LIMIT 1",
    )
    .bind(actual_instance.instance)
    .fetch_one(pool)
    .await
    .expect("completed cooking result for browser inspection");
    // The browser must exercise the approval command against a real
    // completed proposal.  The fault-recovery requests in the joined
    // scenario intentionally remain open after their provenance
    // inspection, so select the newest such proposal instead of
    // fabricating a pending row for the UI fixture.
    let (pending_run_id, pending_proposal_id): (uuid::Uuid, uuid::Uuid) = sqlx::query_as(
        "SELECT run.id, proposal.id
                   FROM runs run
                   JOIN review_proposals proposal ON proposal.run_id = run.id
                  WHERE run.instance_id = $1
                    AND proposal.state IN ('open', 'approval_requested')
                    AND EXISTS (
                        SELECT 1 FROM run_results result
                         WHERE result.run_id = run.id
                           AND result.state = 'completed'
                           AND result.result_commit IS NOT NULL
                    )
                  ORDER BY proposal.created_at DESC, proposal.id DESC
                  LIMIT 1",
    )
    .bind(actual_instance.instance)
    .fetch_one(pool)
    .await
    .expect("open cooking result proposal for browser approval");
    let proposal_id: uuid::Uuid =
        sqlx::query_scalar("SELECT id FROM review_proposals WHERE run_id = $1 ORDER BY id LIMIT 1")
            .bind(completed_run_id)
            .fetch_one(pool)
            .await
            .expect("completed cooking review proposal");
    let retry_run_ids_before: Vec<uuid::Uuid> = sqlx::query_scalar(
        "SELECT run_id FROM run_requests WHERE retry_of_run_id = $1 ORDER BY run_id",
    )
    .bind(
        retry_fixture
            .as_ref()
            .expect("browser retry source run fixture")
            .source_run_id,
    )
    .fetch_all(pool)
    .await
    .expect("browser retry ancestry before browser actions");
    let post_path = format!(
        "{}.post.json",
        env::var("HEPHAESTUS_COOKING_BROWSER_FIXTURE_OUTPUT")
            .expect("cooking browser fixture output path")
    );
    let post_fixture = serde_json::json!({
        "organization_id": organization_id,
        "project_id": project.id,
        "repository_id": builds.gateway.repository_id,
        "release_id": builds.gateway.release_id,
        "release_agent_id": builds.agent.release_agent_id,
        "instance_id": instance.instance_id,
        "mailbox_id": instance.mailbox_id,
        "gateway_id": sqlx::query_scalar::<_, uuid::Uuid>(
            "SELECT binding.gateway_id
                   FROM gateway_mailbox_bindings binding
                   JOIN gateway_mailbox_binding_grants grant_row
                     ON grant_row.binding_id = binding.id
                  WHERE grant_row.id = $1",
        )
        .bind(actual_fixture.grant_id)
        .fetch_one(pool)
        .await
        .expect("post-operation cooking gateway id"),
        "completed_run_id": completed_run_id,
        "result_commit": result_commit,
        "result_ref": result_ref,
        "target_ref": target_ref,
        "proposal_id": proposal_id,
        "pending_run_id": pending_run_id,
        "pending_proposal_id": pending_proposal_id,
        "retry_source_run_id": retry_fixture
            .as_ref()
            .expect("browser retry source run fixture")
            .source_run_id,
        "migration_update_id": update_sequence.as_ref().map(|s| s.migration.update_id),
        "abnormal_update_id": update_sequence.as_ref().map(|s| s.abnormal.update_id),
        "abnormal_candidate_revision_id": update_sequence
            .as_ref()
            .map(|s| s.abnormal.candidate_revision_id),
        "abnormal_release_agent_id": browser_abnormal_release_agent_id,
        "browser_model_rule_id": cooking_update_rule_ids
            .map(|(_, _, _, browser)| browser.model),
        "browser_relay_rule_id": cooking_update_rule_ids
            .map(|(_, _, _, browser)| browser.relay),
        "browser_rule_copies": cooking_update_rule_ids.map(|(migration, _, _, browser)| {
            vec![
                serde_json::json!({
                    "source_rule_id": migration.model,
                    "candidate_rule_id": browser.model,
                }),
                serde_json::json!({
                    "source_rule_id": migration.relay,
                    "candidate_rule_id": browser.relay,
                }),
            ]
        }),
    });
    tokio::fs::write(
        &post_path,
        serde_json::to_vec_pretty(&post_fixture).expect("post-operation browser fixture JSON"),
    )
    .await
    .expect("write post-operation browser fixture JSON");
    let issuer =
        env::var("HEPHAESTUS_COOKING_BROWSER_OIDC_ISSUER").expect("cooking browser OIDC issuer");
    let script =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scripts/run-ui-e2e-external.sh");
    let browser_timer = WorkloadPhaseTimer::start("browser-post-operation", workload_phase_timing);
    let status = tokio::process::Command::new(script)
        .env("HEPHAESTUS_E2E_COOKING_FIXTURE", &post_path)
        .env("HEPHAESTUS_E2E_EXTERNAL_DATABASE_URL", database_url)
        .env(
            "HEPHAESTUS_E2E_EXTERNAL_RPC_ENDPOINT",
            restarted.http_addr().to_string(),
        )
        .env(
            "HEPHAESTUS_E2E_EXTERNAL_RPC_SECRET",
            "golden-internal-command-token-with-sufficient-entropy",
        )
        .env("HEPHAESTUS_E2E_EXTERNAL_OIDC_ISSUER", issuer)
        .env("HEPHAESTUS_E2E_COOKING_PHASE", "post-operation")
        .env("HEPHAESTUS_E2E_BROWSER_RUNNER", "legacy")
        .status()
        .await;
    browser_timer.finish(status.as_ref().is_ok_and(std::process::ExitStatus::success));
    let status = status.expect("run cooking post-operation browser E2E");
    assert!(
        status.success(),
        "cooking post-operation browser E2E failed: {status}"
    );
    // Retry is a separate run command. Do not tear down the daemon
    // while its guest is still cleaning up; otherwise the subsequent
    // NATS and state-volume scans could miss a real active resource.
    let retry_run_ids_after: Vec<uuid::Uuid> = sqlx::query_scalar(
        "SELECT run_id FROM run_requests WHERE retry_of_run_id = $1 ORDER BY run_id",
    )
    .bind(
        retry_fixture
            .as_ref()
            .expect("browser retry source run fixture")
            .source_run_id,
    )
    .fetch_all(pool)
    .await
    .expect("browser retry ancestry after browser actions");
    assert_eq!(
        retry_run_ids_after.len(),
        retry_run_ids_before.len() + 1,
        "browser retry creates exactly one request for the selected source run"
    );
    assert!(
        retry_run_ids_before
            .iter()
            .all(|id| retry_run_ids_after.contains(id))
    );
    let new_retry_ids: Vec<_> = retry_run_ids_after
        .iter()
        .filter(|id| !retry_run_ids_before.contains(id))
        .copied()
        .collect();
    assert_eq!(new_retry_ids.len(), 1);
    let retry_run_id = new_retry_ids[0];
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    loop {
        let (state, outcome): (String, Option<String>) =
            sqlx::query_as("SELECT state, outcome FROM runs WHERE id = $1")
                .bind(retry_run_id)
                .fetch_one(pool)
                .await
                .expect("poll exact browser retry run");
        if state == "cleaned_up" {
            assert_eq!(
                outcome.as_deref(),
                Some("succeeded"),
                "browser retry succeeds for the dedicated forge source"
            );
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "browser retry run did not reach terminal cleanup"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    if let Some(sequence) = update_sequence {
        let abnormal_release_agent_id =
            browser_abnormal_release_agent_id.expect("browser abnormal release agent");
        let state: (String, String, bool) = sqlx::query_as(
            "SELECT update_record.state, instance.state, instance.run_gate_open
                   FROM agent_updates update_record
                   JOIN agent_instances instance ON instance.id = update_record.instance_id
                  WHERE update_record.id = $1",
        )
        .bind(sequence.abnormal.update_id)
        .fetch_one(pool)
        .await
        .expect("browser recovery update state");
        assert_eq!(
            state,
            (
                String::from("rejected"),
                String::from("update_rejected"),
                true
            )
        );
        let browser_updates: i64 = sqlx::query_scalar(
            "SELECT count(*)
                   FROM agent_updates update_record
                   JOIN agent_instance_revisions candidate
                     ON candidate.id = update_record.candidate_revision_id
                  WHERE update_record.instance_id = $1
                    AND candidate.release_agent_id = $2
                    AND update_record.state = 'rejected'",
        )
        .bind(actual_instance.instance)
        .bind(abnormal_release_agent_id)
        .fetch_one(pool)
        .await
        .expect("browser-created abnormal update state");
        assert_eq!(
            browser_updates, 2,
            "both abnormal updates must be recovered"
        );
    }
}
