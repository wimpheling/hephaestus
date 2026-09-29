use super::*;

pub struct ForgeSourceFixture {
    pub input_commit: String,
    pub run_id: uuid::Uuid,
}

#[derive(Clone, Copy)]
pub enum ForgeSourceContent {
    GoldenAgent,
    CookingBlog,
}

// Keep the exact Git, request, result, and proposal assertions together so
// the browser retry fixture cannot accidentally lose one provenance boundary.
#[allow(
    clippy::cognitive_complexity,
    clippy::too_many_arguments,
    clippy::too_many_lines
)]
pub async fn create_forge_source_run(
    pool: &sqlx::PgPool,
    running: &hephaestus_app::RunningHephaestus,
    root: &Path,
    repository_id: uuid::Uuid,
    instance_id: uuid::Uuid,
    token: &str,
    libkrun_e2e: bool,
    require_review_proposal: bool,
    source_content: ForgeSourceContent,
) -> ForgeSourceFixture {
    let source = root.join("source");
    tokio::fs::create_dir(&source)
        .await
        .expect("source repository");
    git(&source, &["init", "--initial-branch=main"]).await;
    git(&source, &["config", "user.name", "Golden Test"]).await;
    git(&source, &["config", "user.email", "golden@example.invalid"]).await;
    tokio::fs::write(source.join("agent.toml"), agent_config())
        .await
        .expect("provider-neutral agent.toml");
    tokio::fs::write(source.join("golden-agent.sh"), GOLDEN_AGENT)
        .await
        .expect("golden agent executable source");
    tokio::fs::write(source.join("input.txt"), "accepted\n")
        .await
        .expect("input file");
    if matches!(source_content, ForgeSourceContent::CookingBlog) {
        cooking::copy_blog(&source).await;
    }
    tokio::fs::create_dir(source.join("reports"))
        .await
        .expect("reports directory");
    tokio::fs::write(source.join("reports/result.txt"), "initial\n")
        .await
        .expect("initial report");
    git(&source, &["add", "."]).await;
    git(&source, &["commit", "-m", "golden agent"]).await;
    if matches!(source_content, ForgeSourceContent::GoldenAgent) {
        let committed_agent = git_output(&source, &["show", "HEAD:agent.toml"]).await;
        assert_eq!(
            committed_agent,
            agent_config().trim(),
            "forge retry source must retain the provider-neutral agent manifest"
        );
        assert!(
            !source.join("heph.images.toml").exists(),
            "forge retry source must not select a repository image"
        );
        assert!(
            committed_agent.contains("image = { key = \"golden-root\" }"),
            "forge retry source must select the registered golden-root image"
        );
    }
    let input_commit = git_output(&source, &["rev-parse", "HEAD"]).await;
    let remote = format!("http://{}/{}", running.http_addr(), repository_id);
    git(&source, &["remote", "add", "origin", &remote]).await;
    sqlx::query("UPDATE agent_instances SET run_gate_open = true WHERE id = $1")
        .bind(instance_id)
        .execute(pool)
        .await
        .expect("open golden push run gate");
    authenticated_git(&source, token, &["push", "origin", "HEAD:refs/heads/main"]).await;
    let run_id: uuid::Uuid = sqlx::query_scalar(
        "SELECT request.run_id
           FROM run_requests request
          WHERE request.repository_id = $1
            AND request.instance_id = $2
            AND request.commit_sha = $3
            AND request.git_ref = 'refs/heads/main'
            AND request.retry_of_run_id IS NULL
            AND request.request_kind = 'instance_normal'
          ORDER BY request.created_at DESC, request.id DESC
          LIMIT 1",
    )
    .bind(repository_id)
    .bind(instance_id)
    .bind(&input_commit)
    .fetch_one(pool)
    .await
    .expect("durable forge run request");
    let runtime_run_id = runtime_types::RunId::from_uuid(run_id);
    let result_wait = running
        .wait_for_run_event(
            runtime_run_id,
            RunEventKind::ResultCompleted,
            Duration::from_secs(if libkrun_e2e { 60 } else { 10 }),
        )
        .await;
    if result_wait.is_err() {
        diagnose_golden_timeout(pool, repository_id, runtime_run_id).await;
    }
    result_wait.expect("persisted forge source result completion");
    let result_commit: Option<String> = sqlx::query_scalar(
        "SELECT result_commit FROM run_results
          WHERE run_id = $1 AND state = 'completed'",
    )
    .bind(run_id)
    .fetch_one(pool)
    .await
    .expect("completed forge source result");
    if require_review_proposal {
        assert!(
            result_commit.is_some(),
            "forge source run must produce a result commit for review retry"
        );
        let proposal_state: String = sqlx::query_scalar(
            "SELECT state FROM review_proposals
              WHERE run_id = $1",
        )
        .bind(run_id)
        .fetch_one(pool)
        .await
        .expect("forge source review proposal");
        assert!(
            matches!(proposal_state.as_str(), "open" | "approval_requested"),
            "forge source proposal remains reviewable"
        );
    }
    ForgeSourceFixture {
        input_commit,
        run_id,
    }
}
