#![allow(unused_imports)]
use super::MODEL_RESPONSE_TEXT;
use super::model::SessionBrokerFixture;
use super::rpc_helpers::git_output_bare;
use super::*;
use forge_domain::GitRef;
use serde_json::Value as JsonValue;
use sqlx::PgPool;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    time::Duration,
};
use uuid::Uuid;
// Keep the cross-store provenance assertions together so a failed acceptance
// identifies one incomplete runtime turn rather than hiding it in helpers.
#[allow(
    clippy::cognitive_complexity,
    clippy::too_many_arguments,
    clippy::too_many_lines
)]
pub(crate) async fn assert_runtime_git_turn(
    pool: &PgPool,
    root: &Path,
    repository_id: Uuid,
    instance_id: Uuid,
    attachment_id: Uuid,
    actor_id: Uuid,
    run_id: Uuid,
    human_commit: &str,
    expected_human_record_id: Option<&str>,
) {
    wait_for_run_succeeded(pool, run_id, Duration::from_secs(120)).await;
    let agent_commit =
        git_output_bare(root, repository_id, &["rev-parse", "refs/heads/main"]).await;
    assert_runtime_git_turn_at_commit(
        pool,
        root,
        repository_id,
        instance_id,
        attachment_id,
        actor_id,
        run_id,
        human_commit,
        &agent_commit,
        expected_human_record_id,
    )
    .await;
}

#[allow(
    clippy::cognitive_complexity,
    clippy::too_many_arguments,
    clippy::too_many_lines
)] // This is one reviewable assertion over a runtime turn at a specified canonical commit.
pub(crate) async fn assert_runtime_git_turn_at_commit(
    pool: &PgPool,
    root: &Path,
    repository_id: Uuid,
    instance_id: Uuid,
    attachment_id: Uuid,
    actor_id: Uuid,
    run_id: Uuid,
    human_commit: &str,
    agent_commit: &str,
    expected_human_record_id: Option<&str>,
) {
    let human_request_attachment: Uuid = sqlx::query_scalar(
        "SELECT request.attachment_id
           FROM run_requests AS request
           JOIN git_ref_updates AS update ON update.receive_id = request.receive_id
           JOIN git_receives AS receive ON receive.id = update.receive_id
          WHERE request.run_id = $1
            AND request.instance_id = $2
            AND request.repository_id = $3
            AND request.commit_sha = $4
            AND request.git_ref = 'refs/heads/main'
            AND request.request_kind = 'instance_normal'
            AND request.attachment_id = $5
            AND receive.repository_id = $3
            AND receive.actor_id = $6
            AND receive.status = 'accepted'
            AND receive.runtime_session_id IS NULL
            AND receive.runtime_attachment_id IS NULL
            AND update.git_ref = 'refs/heads/main'
            AND update.new_commit = $4
          LIMIT 1",
    )
    .bind(run_id)
    .bind(instance_id)
    .bind(repository_id)
    .bind(human_commit)
    .bind(attachment_id)
    .bind(actor_id)
    .fetch_one(pool)
    .await
    .expect("exact human Git receive/run provenance");
    assert_eq!(human_request_attachment, attachment_id);

    assert_eq!(
        git_output_bare(
            root,
            repository_id,
            &["rev-parse", &format!("{agent_commit}^")]
        )
        .await,
        human_commit,
        "canonical agent commit must be directly based on the human input commit"
    );
    let changed = git_output_bare(
        root,
        repository_id,
        &[
            "diff-tree",
            "--no-commit-id",
            "--name-only",
            "-r",
            agent_commit,
        ],
    )
    .await;
    let paths = changed.lines().collect::<Vec<_>>();
    assert!(
        !paths.is_empty(),
        "agent commit must publish session output"
    );
    assert!(paths.iter().all(|path| {
        path.starts_with(".heph/session/v1/records/agent/agent%3Areference-chat/")
            || path.starts_with(".heph/session/v1/context/agent%3Areference-chat/")
    }));

    let agent_record_paths = paths
        .iter()
        .copied()
        .filter(|path| path.starts_with(".heph/session/v1/records/agent/"))
        .collect::<Vec<_>>();
    assert_eq!(
        agent_record_paths.len(),
        1,
        "agent commit must publish exactly one assistant response"
    );
    let context_paths = paths
        .iter()
        .copied()
        .filter(|path| path.starts_with(".heph/session/v1/context/"))
        .collect::<Vec<_>>();
    assert_eq!(
        context_paths,
        [".heph/session/v1/context/agent%3Areference-chat/last_response.json"],
        "agent commit must publish exactly the response context entry"
    );

    let human_paths = git_output_bare(
        root,
        repository_id,
        &[
            "diff-tree",
            "--no-commit-id",
            "--name-only",
            "-r",
            human_commit,
        ],
    )
    .await
    .lines()
    .filter(|path| path.starts_with(".heph/session/v1/records/human/"))
    .map(str::to_owned)
    .collect::<Vec<_>>();
    assert_eq!(
        human_paths.len(),
        1,
        "human commit must publish exactly one canonical input record"
    );
    let human_record_json = git_output_bare(
        root,
        repository_id,
        &["show", &format!("{human_commit}:{}", human_paths[0])],
    )
    .await;
    let human_record: JsonValue =
        serde_json::from_str(&human_record_json).expect("canonical human record JSON");
    let human_record_id = human_record["record_id"]
        .as_str()
        .expect("canonical human record ID");
    assert_eq!(
        human_record_id,
        human_paths[0]
            .strip_prefix(".heph/session/v1/records/human/")
            .and_then(|path| path.strip_suffix(".json"))
            .expect("canonical human record path")
            .replace("%3A", ":")
    );
    if let Some(expected_human_record_id) = expected_human_record_id {
        assert_eq!(human_record_id, expected_human_record_id);
    }

    let agent_record_json = git_output_bare(
        root,
        repository_id,
        &["show", &format!("{agent_commit}:{}", agent_record_paths[0])],
    )
    .await;
    let agent_record: JsonValue =
        serde_json::from_str(&agent_record_json).expect("canonical agent record JSON");
    assert_eq!(agent_record["protocol"], "heph.session-chat");
    assert_eq!(agent_record["version"], 1);
    assert_eq!(agent_record["kind"], "assistant_message");
    assert_eq!(agent_record["actor"]["id"], "agent:reference-chat");
    assert_eq!(agent_record["actor"]["role"], "agent");
    assert_eq!(agent_record["participant_id"], "agent:reference-chat");
    assert_eq!(agent_record["in_reply_to"], human_record_id);
    assert_eq!(agent_record["content"]["kind"], "text");
    assert_eq!(agent_record["content"]["text"], MODEL_RESPONSE_TEXT);
    Uuid::parse_str(
        agent_record["correlation_id"]
            .as_str()
            .expect("canonical agent correlation ID"),
    )
    .expect("canonical agent correlation UUID");

    let context_json = git_output_bare(
        root,
        repository_id,
        &["show", &format!("{agent_commit}:{}", context_paths[0])],
    )
    .await;
    let context: JsonValue = serde_json::from_str(&context_json).expect("canonical context JSON");
    assert_eq!(context["agent_id"], "agent:reference-chat");
    assert_eq!(context["key"], "last_response");
    assert_eq!(context["value"], MODEL_RESPONSE_TEXT);

    let (runtime_receive_id, runtime_session_id, runtime_attachment_id): (Uuid, Uuid, Uuid) =
        sqlx::query_as(
            "SELECT receive.id, receive.runtime_session_id, receive.runtime_attachment_id
               FROM git_ref_updates AS update
               JOIN git_receives AS receive ON receive.id = update.receive_id
              WHERE receive.repository_id = $1
                AND receive.status = 'accepted'
                AND receive.runtime_session_id IS NOT NULL
                AND receive.runtime_attachment_id IS NOT NULL
                AND update.git_ref = 'refs/heads/main'
                AND update.old_commit = $2
                AND update.new_commit = $3
              LIMIT 1",
        )
        .bind(repository_id)
        .bind(human_commit)
        .bind(agent_commit)
        .fetch_one(pool)
        .await
        .expect("runtime-authenticated agent Git receive provenance");
    assert_eq!(runtime_attachment_id, attachment_id);

    let (session_run_id, session_instance_id, session_attachment_id): (Uuid, Uuid, Uuid) =
        sqlx::query_as(
            "SELECT session.run_id, session.instance_id, session.attachment_id
               FROM runtime_authority_sessions AS session
              WHERE session.id = $1
                AND session.run_id = $2
                AND session.instance_id = $3
                AND session.attachment_id IS NOT NULL",
        )
        .bind(runtime_session_id)
        .bind(run_id)
        .bind(instance_id)
        .fetch_one(pool)
        .await
        .expect("runtime session immutable provenance");
    assert_eq!(session_run_id, run_id);
    assert_eq!(session_instance_id, instance_id);
    assert_eq!(session_attachment_id, attachment_id);
    let (provenance_revision, run_revision, target_repository, target_ref, target_commit): (
        Uuid,
        Uuid,
        Uuid,
        String,
        String,
    ) = sqlx::query_as(
        "SELECT provenance.instance_revision_id, run.instance_revision_id
                , provenance.target_repository_id, provenance.target_ref
                , provenance.target_commit
           FROM run_instance_provenance AS provenance
           JOIN runs AS run ON run.id = provenance.run_id
          WHERE provenance.run_id = $1",
    )
    .bind(run_id)
    .fetch_one(pool)
    .await
    .expect("immutable run revision provenance");
    assert_eq!(provenance_revision, run_revision);
    assert_eq!(target_repository, repository_id);
    assert_eq!(target_ref, "refs/heads/main");
    assert_eq!(target_commit, human_commit);
    let runtime_request_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM run_requests WHERE receive_id = $1")
            .bind(runtime_receive_id)
            .fetch_one(pool)
            .await
            .expect("runtime originating receive request count");
    assert_eq!(
        runtime_request_count, 0,
        "originating runtime attachment must not recursively trigger itself"
    );
}
