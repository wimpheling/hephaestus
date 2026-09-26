use super::SessionBrokerFixture;
use super::browser_setup::run_session_chat_browser;
use super::rpc_helpers::git_output_bare;
use super::*;
use hephaestus_app::RunningHephaestus;
use serde_json::Value as JsonValue;
use sqlx::PgPool;
use std::{
    collections::{HashMap, HashSet},
    path::Path,
    time::Duration,
};
use tokio::process::Command;
use uuid::Uuid;
// Keep the concurrency assertions together so each production boundary is checked once.
#[allow(
    clippy::cognitive_complexity,
    clippy::too_many_arguments,
    clippy::too_many_lines
)]
pub(crate) async fn exercise_browser_concurrency(
    pool: &PgPool,
    database_url: &str,
    running: &RunningHephaestus,
    root: &Path,
    broker: SessionBrokerFixture,
    project: ProjectId,
    actor_id: Uuid,
    release_agent_id: Uuid,
    repository_id: Uuid,
    instance_id: Uuid,
    revision_id: Uuid,
    attachment_id: Uuid,
    installation_id: Uuid,
    generation_id: Uuid,
    session_id: Uuid,
) -> SessionBrokerFixture {
    let before_head = git_output_bare(root, repository_id, &["rev-parse", "refs/heads/main"]).await;
    let before_record_paths = git_output_bare(
        root,
        repository_id,
        &["ls-tree", "-r", "--name-only", before_head.trim()],
    )
    .await
    .lines()
    .filter(|path| path.starts_with(".heph/session/v1/records/"))
    .map(str::to_owned)
    .collect::<Vec<_>>();
    let mut before_record_blobs = HashMap::with_capacity(before_record_paths.len());
    for path in before_record_paths {
        let content = git_output_bare_bytes(
            root,
            repository_id,
            &["show", &format!("{before_head}:{path}")],
        )
        .await;
        before_record_blobs.insert(path, content);
    }
    let before_receive_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM git_receives WHERE repository_id = $1 AND status = 'accepted'",
    )
    .bind(repository_id)
    .fetch_one(pool)
    .await
    .expect("concurrency accepted receive baseline");
    let before_run_request_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM run_requests WHERE instance_id = $1 AND repository_id = $2",
    )
    .bind(instance_id)
    .bind(repository_id)
    .fetch_one(pool)
    .await
    .expect("concurrency run request baseline");

    eprintln!("HEPH_SESSION_CHAT_BROWSER stage=concurrency-started");
    run_session_chat_browser(
        database_url,
        running,
        project,
        release_agent_id,
        Uuid::nil(),
        SessionChatBrowserMode::Concurrent {
            repository_id,
            installation_id,
            generation_id,
            actor_id,
            initial_transcript_count: 6,
            initial_agent_count: 3,
        },
    )
    .await;
    broker.wait_for_observed(5).await;
    let requests = broker.observed_snapshot();
    assert_eq!(
        requests.len(),
        5,
        "full session-chat journey must make five model turns"
    );
    let concurrent_requests = &requests[3..];
    assert_eq!(concurrent_requests.len(), 2);
    let mut concurrent_record_ids = HashSet::new();
    for request in concurrent_requests {
        assert_eq!(request.session_id, session_id);
        assert!(concurrent_record_ids.insert(request.record_id));
        let message = request
            .messages
            .last()
            .expect("concurrent model request human message");
        assert_eq!(message.record_id, request.record_id);
        assert_eq!(message.role, "user");
    }

    let head = git_output_bare(root, repository_id, &["rev-parse", "refs/heads/main"]).await;
    assert_ne!(
        head, before_head,
        "concurrency must persist a new canonical head"
    );
    let repository_path = root
        .join("repositories")
        .join(format!("{repository_id}.git"));
    let ancestry = Command::new("git")
        .arg(format!("--git-dir={}", repository_path.display()))
        .args(["merge-base", "--is-ancestor", &before_head, &head])
        .status()
        .await
        .expect("check concurrency Git ancestry");
    assert!(
        ancestry.success(),
        "concurrency must preserve the pre-race history"
    );
    for (path, expected) in &before_record_blobs {
        assert_eq!(
            git_output_bare_bytes(root, repository_id, &["show", &format!("{head}:{path}")]).await,
            expected.as_slice(),
            "pre-concurrency record blob changed: {path}"
        );
    }

    let paths = git_output_bare(
        root,
        repository_id,
        &["ls-tree", "-r", "--name-only", head.trim()],
    )
    .await
    .lines()
    .map(str::to_owned)
    .collect::<Vec<_>>();
    let human_paths = paths
        .iter()
        .filter(|path| path.starts_with(".heph/session/v1/records/human/"))
        .cloned()
        .collect::<Vec<_>>();
    let agent_paths = paths
        .iter()
        .filter(|path| path.starts_with(".heph/session/v1/records/agent/"))
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(
        human_paths.len(),
        5,
        "concurrency must retain five human records"
    );
    assert_eq!(
        agent_paths.len(),
        5,
        "concurrency must retain five assistant records"
    );

    let mut human_record_paths = HashMap::with_capacity(human_paths.len());
    for path in &human_paths {
        let record: JsonValue = serde_json::from_str(
            &git_output_bare(root, repository_id, &["show", &format!("{head}:{path}")]).await,
        )
        .expect("concurrency human record JSON");
        let record_id = Uuid::parse_str(
            record["record_id"]
                .as_str()
                .expect("concurrency human record ID"),
        )
        .expect("concurrency human record UUID");
        assert_eq!(record["kind"], "user_message");
        human_record_paths.insert(record_id, path.clone());
    }
    let mut agent_records = HashMap::with_capacity(agent_paths.len());
    for path in &agent_paths {
        let record: JsonValue = serde_json::from_str(
            &git_output_bare(root, repository_id, &["show", &format!("{head}:{path}")]).await,
        )
        .expect("concurrency assistant record JSON");
        let record_id = Uuid::parse_str(
            record["record_id"]
                .as_str()
                .expect("concurrency assistant record ID"),
        )
        .expect("concurrency assistant record UUID");
        assert_eq!(record["kind"], "assistant_message");
        assert_eq!(record["content"]["text"], MODEL_RESPONSE_TEXT);
        agent_records.insert(record_id, (path.clone(), record));
    }

    let mut run_ids = Vec::with_capacity(concurrent_requests.len());
    for request in concurrent_requests {
        let human_path = human_record_paths
            .get(&request.record_id)
            .expect("concurrent model request human record");
        let human_commit = canonical_record_commit(root, repository_id, human_path).await;
        let ancestor = Command::new("git")
            .arg(format!("--git-dir={}", repository_path.display()))
            .args(["merge-base", "--is-ancestor", &before_head, &human_commit])
            .status()
            .await
            .expect("check concurrent human ancestry");
        assert!(
            ancestor.success(),
            "concurrent human commit must descend from the race head"
        );
        let agent_entry = agent_records
            .values()
            .find(|entry| entry.1["in_reply_to"] == request.record_id.to_string())
            .expect("concurrent assistant response");
        let agent_path = &agent_entry.0;
        let agent_record = &agent_entry.1;
        assert_eq!(agent_record["in_reply_to"], request.record_id.to_string());
        let agent_commit = canonical_record_commit(root, repository_id, agent_path).await;
        assert_eq!(
            git_output_bare(
                root,
                repository_id,
                &["rev-parse", &format!("{agent_commit}^")]
            )
            .await,
            human_commit,
            "concurrent assistant commit must directly parent its human commit"
        );
        let expected_human_record_id = request.record_id.to_string();
        let run_id = accepted_normal_run_id(
            pool,
            repository_id,
            instance_id,
            attachment_id,
            actor_id,
            &human_commit,
        )
        .await;
        let revision_matches: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM run_requests
              WHERE run_id = $1 AND instance_revision_id = $2 AND attachment_id = $3",
        )
        .bind(run_id)
        .bind(revision_id)
        .bind(attachment_id)
        .fetch_one(pool)
        .await
        .expect("concurrency immutable revision provenance");
        assert_eq!(revision_matches, 1);
        wait_for_run_succeeded(pool, run_id, Duration::from_secs(120)).await;
        assert_eq!(accepted_runtime_receive_count(pool, run_id).await, 1);
        assert_runtime_git_turn_at_commit(
            pool,
            root,
            repository_id,
            instance_id,
            attachment_id,
            actor_id,
            run_id,
            &human_commit,
            &agent_commit,
            Some(&expected_human_record_id),
        )
        .await;
        run_ids.push(run_id);
    }

    let after_receive_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM git_receives WHERE repository_id = $1 AND status = 'accepted'",
    )
    .bind(repository_id)
    .fetch_one(pool)
    .await
    .expect("concurrency accepted receive count");
    assert_eq!(
        after_receive_count,
        before_receive_count + 4,
        "concurrency must persist two human and two runtime accepted receives"
    );
    let after_run_request_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM run_requests WHERE instance_id = $1 AND repository_id = $2",
    )
    .bind(instance_id)
    .bind(repository_id)
    .fetch_one(pool)
    .await
    .expect("concurrency run request count");
    assert_eq!(
        after_run_request_count,
        before_run_request_count + 2,
        "stale concurrency attempt must not create a run request"
    );
    let runtime_receive_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM git_receives AS receive
          JOIN runtime_authority_sessions AS session
            ON session.id = receive.runtime_session_id
         WHERE session.run_id = ANY($1) AND receive.status = 'accepted'",
    )
    .bind(&run_ids)
    .fetch_one(pool)
    .await
    .expect("concurrency runtime receive count");
    assert_eq!(runtime_receive_count, 2);
    eprintln!("HEPH_SESSION_CHAT_BROWSER stage=concurrency-canonical-validation-passed turns=5");
    broker
}
