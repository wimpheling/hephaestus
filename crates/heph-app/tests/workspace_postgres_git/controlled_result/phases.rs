use super::*;

async fn prepare_initial(ctx: &ControlledFixture) -> workspace_domain::PublishedResult {
    let prepared = ctx
        .manager
        .prepare(&ctx.run)
        .await
        .expect("prepare exact commit");
    assert_eq!(prepared.mounts.len(), 2);
    let source = prepared
        .mounts
        .iter()
        .find(|mount| mount.tag == "repository-source")
        .expect("source mount");
    let writable = prepared
        .mounts
        .iter()
        .find(|mount| mount.tag == "repository-work")
        .expect("work mount");
    assert!(source.read_only);
    assert!(!writable.read_only);
    assert_eq!(
        tokio::fs::read_to_string(source.host_path.join("input.txt"))
            .await
            .expect("materialized source"),
        "accepted\n"
    );
    tokio::fs::write(writable.host_path.join("input.txt"), "agent edit\n")
        .await
        .expect("edit writable workspace");
    tokio::fs::write(
        writable.host_path.join("reports/result.txt"),
        "durable report\n",
    )
    .await
    .expect("edit declared artifact");
    assert_eq!(
        tokio::fs::read_to_string(source.host_path.join("input.txt"))
            .await
            .expect("source remains readable"),
        "accepted\n",
        "writable workspace mutation changed the immutable source tree"
    );

    let published = ctx
        .manager
        .finalize(&ctx.run, "agent result")
        .await
        .expect("publish controlled result")
        .expect("workspace result");
    assert_eq!(
        tokio::fs::read_to_string(source.host_path.join("input.txt"))
            .await
            .expect_err("sealed source path was cleaned")
            .kind(),
        std::io::ErrorKind::NotFound
    );
    published
}

async fn assert_published(ctx: &ControlledFixture, published: &workspace_domain::PublishedResult) {
    assert_eq!(
        git_output_bare(&ctx.bare, &["rev-parse", &published.result_ref]).await,
        published.result_commit
    );
    assert_eq!(
        git_output_bare(&ctx.bare, &["rev-parse", "refs/heads/main"]).await,
        ctx.newer.as_str()
    );
    assert_eq!(
        git_output_bare(
            &ctx.bare,
            &["rev-parse", &format!("{}^", published.result_commit)]
        )
        .await,
        ctx.accepted.as_str()
    );
    assert_eq!(
        git_output_bare(
            &ctx.bare,
            &["show", &format!("{}:input.txt", published.result_commit)]
        )
        .await,
        "agent edit"
    );
    let artifact_kinds: Vec<String> = sqlx::query_scalar(
        "SELECT kind FROM result_artifacts artifact
     JOIN run_results result ON result.id = artifact.result_id
     WHERE result.run_id = $1 ORDER BY kind",
    )
    .bind(ctx.run.id.as_uuid())
    .fetch_all(&ctx.pool)
    .await
    .expect("result artifacts");
    assert_eq!(
        artifact_kinds,
        ["declared_file", "exit", "logs", "manifest", "patch"]
    );
    let manifest_key: String = sqlx::query_scalar(
        "SELECT artifact.storage_key
     FROM result_artifacts artifact
     JOIN run_results result ON result.id = artifact.result_id
     WHERE result.run_id = $1 AND artifact.kind = 'manifest'",
    )
    .bind(ctx.run.id.as_uuid())
    .fetch_one(&ctx.pool)
    .await
    .expect("manifest storage key");
    let manifest: serde_json::Value = serde_json::from_slice(
        &tokio::fs::read(ctx.root.join("artifacts").join(manifest_key))
            .await
            .expect("durable manifest"),
    )
    .expect("manifest JSON");
    assert_eq!(
        manifest["result_commit"].as_str(),
        Some(published.result_commit.as_str())
    );
    assert_eq!(
        manifest["input_commit"].as_str(),
        Some(ctx.accepted.as_str())
    );
    let result_state: String =
        sqlx::query_scalar("SELECT state FROM run_results WHERE run_id = $1")
            .bind(ctx.run.id.as_uuid())
            .fetch_one(&ctx.pool)
            .await
            .expect("result state");
    assert_eq!(result_state, "completed");
}

// Recovery and rejection share the same persisted result graph; keeping these
// phases together preserves the crash-recovery ordering under test.
#[allow(clippy::too_many_lines)]
async fn recover_and_reject(
    ctx: &ControlledFixture,
    published: &workspace_domain::PublishedResult,
) {
    let duplicate = ctx
        .manager
        .finalize(&ctx.run, "ignored retry message")
        .await
        .expect("idempotent finalize")
        .expect("existing result");
    assert_eq!(duplicate, published.clone());
    git(&ctx.bare, &["update-ref", "-d", &published.result_ref]).await;
    sqlx::query(
        "UPDATE run_results
     SET state = 'prepared', published_at = NULL, completed_at = NULL
     WHERE run_id = $1",
    )
    .bind(ctx.run.id.as_uuid())
    .execute(&ctx.pool)
    .await
    .expect("simulate crash before ref publication");
    assert_eq!(ctx.manager.recover().await.expect("recover result CAS"), 1);
    assert_eq!(
        git_output_bare(&ctx.bare, &["rev-parse", &published.result_ref]).await,
        published.result_commit
    );

    let second_receive = ctx
        .forge
        .accept_receive(
            &ctx.repository,
            ReceiveId::new(),
            "workspace-test",
            &[RefUpdate {
                git_ref: GitRef::parse("refs/heads/main").expect("main ref"),
                old_commit: Some(ctx.accepted.clone()),
                new_commit: Some(ctx.newer.clone()),
            }],
        )
        .await
        .expect("accept second receive");
    let second_command = second_receive
        .run_requests
        .first()
        .expect("second run request")
        .command
        .clone();
    let second_run = ctx
        .runs
        .create_run(&second_command)
        .await
        .expect("create second run")
        .run;
    let second_workspace = ctx
        .manager
        .prepare(&second_run)
        .await
        .expect("prepare second exact commit");
    let second_source = second_workspace
        .mounts
        .iter()
        .find(|mount| mount.tag == "repository-source")
        .expect("second source mount");
    assert_eq!(
        tokio::fs::read_to_string(second_source.host_path.join("input.txt"))
            .await
            .expect("second materialized source"),
        "newer branch tip\n"
    );
    let socket = UnixListener::bind(
        second_workspace
            .mounts
            .iter()
            .find(|mount| mount.tag == "repository-work")
            .expect("second writable mount")
            .host_path
            .join("guest.sock"),
    )
    .expect("guest-created socket");
    drop(socket);
    assert!(
        ctx.manager
            .finalize(&second_run, "unsafe result")
            .await
            .is_err(),
        "a guest-created socket was imported"
    );
    let rejected_ref: String =
        sqlx::query_scalar("SELECT result_ref FROM run_results WHERE run_id = $1")
            .bind(second_run.id.as_uuid())
            .fetch_one(&ctx.pool)
            .await
            .expect("rejected result ref");
    assert!(
        !git_ref_exists(&ctx.bare, &rejected_ref).await,
        "a rejected result ref was published"
    );
    ctx.manager
        .abandon(second_run.id)
        .await
        .expect("clean second ephemeral workspace");
    assert!(
        ctx.root
            .join("artifacts")
            .join(ctx.run.id.to_string())
            .is_dir(),
        "cleaning a second workspace removed the first run's durable artifacts"
    );
}

pub async fn run_case(ctx: ControlledFixture) {
    let published = prepare_initial(&ctx).await;
    assert_published(&ctx, &published).await;
    recover_and_reject(&ctx, &published).await;
}
