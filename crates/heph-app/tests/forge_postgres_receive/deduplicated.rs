use super::*;

#[tokio::test]
#[serial]
async fn persists_exact_config_and_deduplicates_receive() {
    let Some((pool, service, repository, temporary)) = fixture().await else {
        return;
    };
    seed_reusable_attachment(&pool, &repository).await;
    let config = valid_config(repository.id.as_uuid());
    let (commit, update) = commit_and_update(&temporary, &repository, &config).await;
    let receive_id = ReceiveId::new();
    let first = service
        .accept_receive(
            &repository,
            receive_id,
            "integration-user",
            std::slice::from_ref(&update),
        )
        .await
        .expect("accepted receive");
    let duplicate = service
        .accept_receive(&repository, receive_id, "integration-user", &[update])
        .await
        .expect("duplicate receive");

    assert_eq!(first.run_requests.len(), 1);
    assert_eq!(duplicate.run_requests, first.run_requests);
    let stored_commit: String = sqlx::query_scalar(
        "SELECT commit_sha FROM agent_config_revisions WHERE repository_id = $1",
    )
    .bind(repository.id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("stored config commit");
    let starts: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM outbox
         WHERE aggregate_type = 'forge' AND subject = $1
           AND aggregate_id = $2",
    )
    .bind(INSTANCE_RUN_REQUESTED_SUBJECT)
    .bind(first.run_requests[0].id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("start command count");
    assert_eq!(stored_commit, commit.as_str());
    assert_eq!(starts, 1);
    let reusable_requests: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM run_requests
         WHERE receive_id = $1 AND request_kind = 'instance_normal'",
    )
    .bind(receive_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("exact reusable request count");
    assert_eq!(reusable_requests, 1);
    let reusable_events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM outbox
         WHERE subject = 'hephaestus.instance.run.requested.v1'
           AND payload->>'receive_id' = $1",
    )
    .bind(receive_id.to_string())
    .fetch_one(&pool)
    .await
    .expect("reusable run event count");
    assert_eq!(reusable_events, 1);

    let work = temporary.path().join("work");
    let invalid = config.replace("version = 2", "version = 99");
    tokio::fs::write(work.join("agent.toml"), invalid)
        .await
        .expect("invalid agent configuration");
    git(&work, &["add", "agent.toml"]).await;
    git(&work, &["commit", "-m", "unsupported config"]).await;
    let invalid_commit =
        CommitSha::parse(git_output(&work, &["rev-parse", "HEAD"]).await).expect("invalid commit");
    let bare = temporary
        .path()
        .join("repositories")
        .join(format!("{}.git", repository.id));
    git(
        &work,
        &[
            "push",
            bare.to_str().expect("UTF-8 bare path"),
            "HEAD:refs/heads/main",
        ],
    )
    .await;
    let invalid_result = service
        .accept_receive(
            &repository,
            ReceiveId::new(),
            "integration-user",
            &[RefUpdate {
                git_ref: GitRef::parse("refs/heads/main").expect("updated ref"),
                old_commit: Some(commit),
                new_commit: Some(invalid_commit),
            }],
        )
        .await
        .expect("invalid configuration receive");
    assert_eq!(invalid_result.invalid_configurations, 1);
    assert_eq!(
        invalid_result.run_requests.len(),
        1,
        "target agent.toml validity must not control attached instances"
    );
    let diagnostic_code: String = sqlx::query_scalar(
        "SELECT diagnostics->0->>'code'
         FROM agent_config_revisions
         WHERE repository_id = $1 AND status = 'invalid'",
    )
    .bind(repository.id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("stored diagnostic");
    assert_eq!(diagnostic_code, "unsupported_version");

    cleanup(&pool, repository).await;
}
