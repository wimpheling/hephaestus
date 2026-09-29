use super::*;

pub async fn assert_results(ctx: &SmartHttpContext) {
    let receive_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM git_receives WHERE repository_id = $1")
            .bind(ctx.repository.id.as_uuid())
            .fetch_one(&ctx.pool)
            .await
            .expect("receive count");
    let update_commit: String = sqlx::query_scalar(
        "SELECT u.new_commit
         FROM git_ref_updates u
         JOIN git_receives r ON r.id = u.receive_id
         WHERE r.repository_id = $1 AND u.git_ref = 'refs/heads/main'",
    )
    .bind(ctx.repository.id.as_uuid())
    .fetch_one(&ctx.pool)
    .await
    .expect("audited commit");
    let revision_commit: String = sqlx::query_scalar(
        "SELECT commit_sha FROM agent_config_revisions
         WHERE repository_id = $1
         ORDER BY created_at DESC, id DESC
         LIMIT 1",
    )
    .bind(ctx.repository.id.as_uuid())
    .fetch_one(&ctx.pool)
    .await
    .expect("configuration revision");
    let run_request_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM run_requests WHERE repository_id = $1")
            .bind(ctx.repository.id.as_uuid())
            .fetch_one(&ctx.pool)
            .await
            .expect("run-request count");
    let start_event_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM outbox
         WHERE aggregate_type = 'forge' AND subject = $1
           AND aggregate_id IN (
               SELECT run_id FROM run_requests WHERE repository_id = $2
           )",
    )
    .bind(RUN_START_SUBJECT)
    .bind(ctx.repository.id.as_uuid())
    .fetch_one(&ctx.pool)
    .await
    .expect("start-event count");
    let run_authorization_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM authorization_audit_events
         WHERE actor_id = $1 AND permission = 'can_execute'",
    )
    .bind(ctx.user_id.as_uuid())
    .fetch_one(&ctx.pool)
    .await
    .expect("run authorization count");

    assert_eq!(receive_count, 2);
    assert_eq!(update_commit, ctx.commit);
    assert_eq!(revision_commit, ctx.runtime_commit);
    assert_eq!(run_request_count, 2);
    assert_eq!(start_event_count, 2);
    assert_eq!(run_authorization_count, 2);
    let calls = ctx.authorizer.calls.lock().await.clone();
    assert!(calls.contains(&GitOperation::Clone));
    assert!(calls.contains(&GitOperation::Fetch));
    assert!(calls.contains(&GitOperation::Push));

    let deletable = ctx
        .repository_service
        .create_repository(
            &ctx.authorizer.identity,
            &CreateRepository {
                project_id: ctx.project.id,
                name: String::from("deletable"),
                default_branch: GitRef::parse("refs/heads/main").expect("default branch"),
                is_public: false,
                agent_runs_enabled: false,
            },
        )
        .await
        .expect("authorized deletable repository");
    let deletable_path = ctx.storage.repository_path(deletable.id);
    ctx.repository_service
        .delete_repository(&ctx.authorizer.identity, deletable.id)
        .await
        .expect("authorized repository deletion");
    assert!(!deletable_path.exists());
}
