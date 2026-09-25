#![allow(unused_imports)]
use super::ObservedModelRequest;
use super::browser_records::{BrowserRecordEvidence, load_browser_records};
use super::*;
use forge_domain::ProjectId;
use sqlx::PgPool;
use std::time::Duration;
use std::{collections::HashSet, path::Path};
use uuid::Uuid;

#[allow(clippy::cognitive_complexity, clippy::too_many_lines)]
// Keep the complete persisted browser session as one auditable assertion.
pub(crate) async fn assert_browser_session(
    pool: &PgPool,
    root: &Path,
    project: ProjectId,
    actor_id: Uuid,
    release_agent_id: Uuid,
    existing_repository_ids: &HashSet<Uuid>,
    requests: Vec<ObservedModelRequest>,
) {
    let BrowserRecordEvidence {
        repository_id,
        instance_id,
        revision_id,
        attachment_id,
        session_id,
        human_records,
        human_record_paths,
        agent_records,
        agent_record_paths,
    } = load_browser_records(
        pool,
        root,
        project,
        release_agent_id,
        existing_repository_ids,
        &requests,
    )
    .await;
    let first = &requests[0];
    let second = &requests[1];
    assert_eq!(first.session_id, session_id);
    assert_eq!(second.session_id, session_id);
    assert_ne!(first.record_id, second.record_id);
    assert!(
        human_records
            .iter()
            .any(|(record_id, _)| *record_id == first.record_id)
    );
    assert!(
        human_records
            .iter()
            .any(|(record_id, _)| *record_id == second.record_id)
    );
    assert_eq!(first.messages.len(), 1);
    assert_eq!(first.messages[0].role, "user");
    assert_eq!(second.messages.len(), 3);
    assert_eq!(second.messages[0].role, "user");
    assert_eq!(second.messages[1].role, "assistant");
    assert_eq!(second.messages[2].role, "user");
    assert_eq!(first.messages[0].record_id, first.record_id);
    assert_eq!(second.messages[0].record_id, first.record_id);
    assert_eq!(second.messages[2].record_id, second.record_id);
    let first_agent = agent_records
        .iter()
        .find(|(_, record)| record["in_reply_to"] == first.record_id.to_string())
        .expect("first assistant response");
    let second_agent = agent_records
        .iter()
        .find(|(_, record)| record["in_reply_to"] == second.record_id.to_string())
        .expect("second assistant response");
    assert_eq!(second.messages[1].record_id, first_agent.0);
    assert_ne!(first_agent.0, second_agent.0);

    let first_human_path = human_record_paths
        .get(&first.record_id)
        .expect("first model request human record path");
    let first_human_commit = canonical_record_commit(root, repository_id, first_human_path).await;
    let initialization_commit = git_output_bare(
        root,
        repository_id,
        &["rev-parse", &format!("{first_human_commit}^")],
    )
    .await;
    let initialization_parent_line = git_output_bare(
        root,
        repository_id,
        &["rev-list", "--parents", "-n", "1", &initialization_commit],
    )
    .await;
    let initialization_parents = initialization_parent_line
        .split_whitespace()
        .collect::<Vec<_>>();
    assert_eq!(
        initialization_parents,
        [initialization_commit.as_str()],
        "session initialization must be the root commit"
    );
    let initialization_paths = git_output_bare(
        root,
        repository_id,
        &["ls-tree", "-r", "--name-only", &initialization_commit],
    )
    .await
    .lines()
    .map(str::to_owned)
    .collect::<Vec<_>>();
    assert_eq!(
        initialization_paths,
        [
            ".heph/session/v1/manifest.json".to_owned(),
            ".heph/session/v1/participants/agent%3Areference-chat.json".to_owned(),
            ".heph/session/v1/participants/release%3Areference-chat.json".to_owned(),
            format!(".heph/session/v1/participants/user%3A{actor_id}.json"),
        ],
        "session initialization must publish only its manifest and participants"
    );
    let initialization_run_id = accepted_normal_run_id(
        pool,
        repository_id,
        instance_id,
        attachment_id,
        actor_id,
        &initialization_commit,
    )
    .await;
    wait_for_run_succeeded(pool, initialization_run_id, Duration::from_secs(120)).await;
    let initialization_runtime_receives =
        accepted_runtime_receive_count(pool, initialization_run_id).await;
    assert_eq!(
        initialization_runtime_receives, 0,
        "idle initialization must not publish a runtime-authenticated receive"
    );

    for request in [first, second] {
        let human_path = human_record_paths
            .get(&request.record_id)
            .expect("model request human record path");
        let human_commit = canonical_record_commit(root, repository_id, human_path).await;
        let agent = agent_records
            .iter()
            .find(|(_, record)| record["in_reply_to"] == request.record_id.to_string())
            .expect("assistant response for each model request");
        let run_id = accepted_normal_run_id(
            pool,
            repository_id,
            instance_id,
            attachment_id,
            actor_id,
            &human_commit,
        )
        .await;
        wait_for_run_succeeded(pool, run_id, Duration::from_secs(120)).await;
        assert_eq!(
            accepted_runtime_receive_count(pool, run_id).await,
            1,
            "each human turn must have one accepted runtime-authenticated receive"
        );
        let agent_path = agent_record_paths
            .get(&agent.0)
            .expect("assistant record path");
        let agent_commit = canonical_record_commit(root, repository_id, agent_path).await;
        assert_eq!(
            git_output_bare(
                root,
                repository_id,
                &["rev-parse", &format!("{agent_commit}^")],
            )
            .await,
            human_commit,
            "assistant commit must be directly based on its human commit"
        );
        let expected_human_record_id = request.record_id.to_string();
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
    }

    // The new-session browser flow asserts an empty transcript immediately
    // after opening the UI; its initialization creates one idle run, and its
    // two explicit Send steps create the only model-bearing runs.
    let run_request_count: i64 = sqlx::query_scalar(
        "SELECT count(*)
           FROM run_requests
          WHERE instance_id = $1 AND repository_id = $2",
    )
    .bind(instance_id)
    .bind(repository_id)
    .fetch_one(pool)
    .await
    .expect("browser session run request count");
    assert_eq!(
        run_request_count, 3,
        "browser initialization must schedule one idle run; two human runs must not recurse"
    );
    let receive_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM git_receives WHERE repository_id = $1")
            .bind(repository_id)
            .fetch_one(pool)
            .await
            .expect("browser session Git receive count");
    assert!(
        receive_count >= 4,
        "browser session must persist both turns and reconnect Git receives"
    );
    if let Ok(diagnostics_dir) = std::env::var("HEPHAESTUS_COOKING_DIAGNOSTICS_DIR") {
        let report = serde_json::json!({
            "mode": "session_chat_new",
            "repository_id": repository_id,
            "instance_id": instance_id,
            "revision_id": revision_id,
            "attachment_id": attachment_id,
            "session_id": session_id,
            "model_requests": requests.iter().map(|request| serde_json::json!({
                "session_id": request.session_id,
                "record_id": request.record_id,
                "message_ids": request.messages.iter().map(|message| message.record_id).collect::<Vec<_>>(),
                "message_roles": request.messages.iter().map(|message| message.role.as_str()).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
            "human_record_ids": human_records.iter().map(|(record_id, _)| record_id).collect::<Vec<_>>(),
            "assistant_record_ids": agent_records.iter().map(|(record_id, _)| record_id).collect::<Vec<_>>(),
            "git_receive_count": receive_count,
        });
        tokio::fs::create_dir_all(&diagnostics_dir)
            .await
            .expect("browser session diagnostics directory");
        tokio::fs::write(
            Path::new(&diagnostics_dir).join("session-chat-browser-report.json"),
            serde_json::to_vec_pretty(&report).expect("browser session diagnostics JSON"),
        )
        .await
        .expect("browser session diagnostics report");
    }
    eprintln!("HEPH_SESSION_CHAT_BROWSER stage=canonical-validation-passed turns=2");
}
