use super::rpc_helpers::git_output_bare;
use super::*;
use sqlx::PgPool;
use std::path::Path;
use uuid::Uuid;
pub(crate) async fn canonical_record_commit(
    root: &Path,
    repository_id: Uuid,
    record_path: &str,
) -> String {
    let commits = git_output_bare(
        root,
        repository_id,
        &[
            "log",
            "--all",
            "--diff-filter=A",
            "--format=%H",
            "--",
            record_path,
        ],
    )
    .await;
    let commits = commits.lines().collect::<Vec<_>>();
    assert_eq!(
        commits.len(),
        1,
        "canonical record must be added by exactly one commit: {record_path}"
    );
    commits[0].to_owned()
}

pub(crate) async fn accepted_normal_run_id(
    pool: &PgPool,
    repository_id: Uuid,
    instance_id: Uuid,
    attachment_id: Uuid,
    actor_id: Uuid,
    commit: &str,
) -> Uuid {
    sqlx::query_scalar(
        "SELECT request.run_id
           FROM run_requests AS request
           JOIN git_ref_updates AS update ON update.receive_id = request.receive_id
           JOIN git_receives AS receive ON receive.id = update.receive_id
          WHERE request.repository_id = $1
            AND request.instance_id = $2
            AND request.commit_sha = $3
            AND request.git_ref = 'refs/heads/main'
            AND request.request_kind = 'instance_normal'
            AND request.attachment_id = $4
            AND receive.repository_id = $1
            AND receive.actor_id = $5
            AND receive.status = 'accepted'
            AND receive.runtime_session_id IS NULL
            AND receive.runtime_attachment_id IS NULL
            AND update.git_ref = 'refs/heads/main'
            AND update.new_commit = $3
          LIMIT 1",
    )
    .bind(repository_id)
    .bind(instance_id)
    .bind(commit)
    .bind(attachment_id)
    .bind(actor_id)
    .fetch_one(pool)
    .await
    .expect("accepted browser session run request")
}

pub(crate) async fn accepted_runtime_receive_count(pool: &PgPool, run_id: Uuid) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*)
           FROM git_receives AS receive
           JOIN runtime_authority_sessions AS session
             ON session.id = receive.runtime_session_id
          WHERE session.run_id = $1
            AND receive.status = 'accepted'",
    )
    .bind(run_id)
    .fetch_one(pool)
    .await
    .expect("runtime receive provenance count")
}
