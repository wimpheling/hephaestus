use super::*;

async fn exercise_valid_and_denied_paths(ctx: &SmartHttpContext, runtime: &RuntimeFixture) {
    git(
        &ctx.source,
        &["remote", "add", "runtime", &runtime.runtime_remote],
    )
    .await;
    git(
        &ctx.source,
        &[
            "remote",
            "add",
            "other-runtime",
            &runtime.other_runtime_remote,
        ],
    )
    .await;
    let wrong_credential = format!(
        "Basic {}",
        BASE64_STANDARD.encode("heph-runtime:heph_git_v1_invalid")
    );
    let wrong_token = git_authenticated_result(
        &ctx.source,
        &["push", "runtime", "HEAD:refs/heads/main"],
        &wrong_credential,
    )
    .await;
    assert!(
        !wrong_token.status.success(),
        "wrong runtime token was accepted"
    );
    assert!(!String::from_utf8_lossy(&wrong_token.stderr).contains("heph_git_v1_invalid"));
    let other_repository_attempt = git_authenticated_result(
        &ctx.source,
        &["push", "other-runtime", "HEAD:refs/heads/main"],
        &runtime.runtime_credential_header,
    )
    .await;
    assert!(
        !other_repository_attempt.status.success(),
        "a runtime credential was accepted for another repository"
    );
    git_authenticated(
        &ctx.source,
        &["push", "runtime", "HEAD:refs/heads/main"],
        &runtime.runtime_credential_header,
    )
    .await;
    tokio::fs::write(ctx.source.join("outside.txt"), "denied\n")
        .await
        .expect("denied runtime change");
    git(&ctx.source, &["add", "outside.txt"]).await;
    git(&ctx.source, &["commit", "-m", "denied runtime path"]).await;
    let denied_path = git_authenticated_result(
        &ctx.source,
        &["push", "runtime", "HEAD:refs/heads/main"],
        &runtime.runtime_credential_header,
    )
    .await;
    assert!(
        !denied_path.status.success(),
        "out-of-scope runtime path was accepted"
    );
    assert!(
        String::from_utf8_lossy(&denied_path.stderr).contains("runtime receive denied"),
        "guarded hook denial was not reported"
    );
    git(&ctx.source, &["reset", "--hard", &ctx.runtime_commit]).await;
}

async fn exercise_ref_and_force_denials(ctx: &SmartHttpContext, runtime: &RuntimeFixture) {
    tokio::fs::create_dir_all(ctx.source.join("sessions"))
        .await
        .expect("runtime ref fixture directory");
    tokio::fs::write(ctx.source.join("sessions/ref-denied.json"), "denied ref\n")
        .await
        .expect("denied runtime ref change");
    git(&ctx.source, &["add", "sessions/ref-denied.json"]).await;
    git(&ctx.source, &["commit", "-m", "denied runtime ref"]).await;
    let denied_ref = git_authenticated_result(
        &ctx.source,
        &["push", "runtime", "HEAD:refs/heads/other"],
        &runtime.runtime_credential_header,
    )
    .await;
    assert!(
        !denied_ref.status.success(),
        "out-of-scope runtime ref was accepted"
    );
    assert!(
        String::from_utf8_lossy(&denied_ref.stderr).contains("runtime receive denied"),
        "ref denial was not reported"
    );
    git(&ctx.source, &["reset", "--hard", &ctx.runtime_commit]).await;
    let denied_delete = git_authenticated_result(
        &ctx.source,
        &["push", "runtime", ":refs/heads/main"],
        &runtime.runtime_credential_header,
    )
    .await;
    assert!(
        !denied_delete.status.success(),
        "runtime branch deletion was accepted"
    );
    assert!(
        String::from_utf8_lossy(&denied_delete.stderr).contains("runtime receive denied"),
        "delete denial was not reported by the receive hook"
    );
    let runtime_commit = &ctx.runtime_commit;
    let baseline_tree = git_output(
        &ctx.source,
        &["rev-parse", &format!("{runtime_commit}^{{tree}}")],
    )
    .await;
    let force_commit = git_output(
        &ctx.source,
        &[
            "commit-tree",
            &baseline_tree,
            "-m",
            "non-fast-forward runtime change",
        ],
    )
    .await;
    let denied_force = git_authenticated_result(
        &ctx.source,
        &[
            "push",
            "runtime",
            &format!("+{force_commit}:refs/heads/main"),
        ],
        &runtime.runtime_credential_header,
    )
    .await;
    assert!(
        !denied_force.status.success(),
        "runtime force push was accepted"
    );
    assert!(
        String::from_utf8_lossy(&denied_force.stderr).contains("runtime receive denied"),
        "force-push denial was not reported by the receive hook"
    );
}

async fn exercise_expired_and_revoked(ctx: &SmartHttpContext, runtime: &RuntimeFixture) {
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    let expired_state: (String, bool) = sqlx::query_as(
        "SELECT status, expires_at < now()
           FROM runtime_authority_sessions WHERE id = $1",
    )
    .bind(runtime.expired_session_id)
    .fetch_one(&ctx.pool)
    .await
    .expect("inspect expired runtime Git session");
    assert_eq!(expired_state, (String::from("active"), true));
    tokio::fs::write(ctx.source.join("runtime.txt"), "expired runtime\n")
        .await
        .expect("expired runtime change");
    git(&ctx.source, &["add", "runtime.txt"]).await;
    git(&ctx.source, &["commit", "-m", "expired runtime session"]).await;
    let expired_header = credential_header(&runtime.expired_credential);
    let expired_authentication = runtime
        .runtime_authenticator
        .authenticate_git(
            Some(&expired_header),
            RequestId::new(),
            ctx.repository.id,
            GitOperation::Push,
        )
        .await;
    assert!(
        expired_authentication.is_err(),
        "expired runtime credential was accepted by the production authenticator"
    );
    let denied_expired = git_authenticated_result(
        &ctx.source,
        &["push", "runtime", "HEAD:refs/heads/main"],
        &expired_header,
    )
    .await;
    assert!(
        !denied_expired.status.success(),
        "expired runtime session was accepted"
    );
    git(&ctx.source, &["reset", "--hard", &ctx.runtime_commit]).await;
    tokio::fs::write(ctx.source.join("runtime.txt"), "revoked runtime\n")
        .await
        .expect("revoked runtime change");
    git(&ctx.source, &["add", "runtime.txt"]).await;
    git(&ctx.source, &["commit", "-m", "revoked runtime session"]).await;
    let revoked_header = credential_header(&runtime.revoked_credential);
    sqlx::query(
        "UPDATE runtime_authority_sessions
            SET status = 'revoked', revoked_at = now(), revocation_reason = 'test'
          WHERE id = $1",
    )
    .bind(runtime.revoked_session_id)
    .execute(&ctx.pool)
    .await
    .expect("revoke runtime Git session");
    let revoked_authentication = runtime
        .runtime_authenticator
        .authenticate_git(
            Some(&revoked_header),
            RequestId::new(),
            ctx.repository.id,
            GitOperation::Push,
        )
        .await;
    assert!(
        revoked_authentication.is_err(),
        "revoked runtime credential was accepted by the production authenticator"
    );
    let denied_revoked = git_authenticated_result(
        &ctx.source,
        &["push", "runtime", "HEAD:refs/heads/main"],
        &revoked_header,
    )
    .await;
    assert!(
        !denied_revoked.status.success(),
        "revoked runtime session was accepted"
    );
    assert_eq!(
        git_output(
            &ctx.storage.repository_path(ctx.repository.id),
            &["rev-parse", "refs/heads/main"],
        )
        .await,
        ctx.runtime_commit,
        "guarded rejection changed canonical ref"
    );
}

async fn assert_runtime_persistence(ctx: &SmartHttpContext, runtime: &RuntimeFixture) {
    let other_receive_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM git_receives WHERE repository_id = $1")
            .bind(ctx.other_repository.id.as_uuid())
            .fetch_one(&ctx.pool)
            .await
            .expect("other repository receive count");
    assert_eq!(
        other_receive_count, 0,
        "wrong-repository attempt reached receive persistence"
    );
    let runtime_receive_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM git_receives
          WHERE runtime_session_id = ANY($1)",
    )
    .bind(vec![
        runtime.runtime_session_id,
        runtime.expired_session_id,
        runtime.revoked_session_id,
    ])
    .fetch_one(&ctx.pool)
    .await
    .expect("runtime receive count");
    assert_eq!(
        runtime_receive_count, 1,
        "a rejected runtime receive was persisted"
    );
    let runtime_run_request_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM run_requests
          WHERE receive_id IN (
              SELECT id FROM git_receives WHERE runtime_session_id = ANY($1)
          )",
    )
    .bind(vec![
        runtime.runtime_session_id,
        runtime.expired_session_id,
        runtime.revoked_session_id,
    ])
    .fetch_one(&ctx.pool)
    .await
    .expect("runtime receive run-request count");
    assert_eq!(
        runtime_run_request_count, 0,
        "runtime receive recursively triggered a run"
    );
    let runtime_receive: (Uuid, Option<Uuid>) = sqlx::query_as(
        "SELECT runtime_session_id, runtime_attachment_id
         FROM git_receives
         WHERE repository_id = $1 AND runtime_session_id = $2",
    )
    .bind(ctx.repository.id.as_uuid())
    .bind(runtime.runtime_session_id)
    .fetch_one(&ctx.pool)
    .await
    .expect("runtime transport receive provenance");
    assert_eq!(
        runtime_receive,
        (runtime.runtime_session_id, Some(runtime.origin_attachment))
    );
    let runtime_run_requests: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM run_requests
         WHERE receive_id = (
             SELECT id FROM git_receives WHERE runtime_session_id = $1
         )",
    )
    .bind(runtime.runtime_session_id)
    .fetch_one(&ctx.pool)
    .await
    .expect("runtime transport run requests");
    assert_eq!(runtime_run_requests, 0);
}

pub async fn exercise_runtime(ctx: &SmartHttpContext, runtime: &RuntimeFixture) {
    exercise_valid_and_denied_paths(ctx, runtime).await;
    exercise_ref_and_force_denials(ctx, runtime).await;
    exercise_expired_and_revoked(ctx, runtime).await;
    assert_runtime_persistence(ctx, runtime).await;
    runtime.runtime_server.abort();
}
