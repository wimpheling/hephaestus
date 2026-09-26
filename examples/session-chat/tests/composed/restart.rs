use super::restart_tail::finish_restart;
use super::*;
use super::{BrowserRestartState, MODEL_RESPONSE_TEXT};
use super::{
    assert_runtime_git_turn_at_commit, canonical_record_commit, git_output_bare,
    git_output_bare_bytes, run_session_chat_browser,
};
use hephaestus_app::RunningHephaestus;
use serde_json::Value as JsonValue;
use sqlx::PgPool;
use std::{collections::HashMap, path::Path, time::Duration};
use time::OffsetDateTime;
use tokio::process::Command;
use uuid::Uuid;

#[allow(clippy::cognitive_complexity, clippy::too_many_lines)]
/// Restarts the browser-backed session against the already-persisted installation.
pub(crate) async fn exercise_browser_restart(
    pool: &PgPool,
    database_url: &str,
    running: &RunningHephaestus,
    root: &Path,
    state: BrowserRestartState<'_>,
    restart_boundary: OffsetDateTime,
) {
    let BrowserRestartState {
        broker,
        project,
        organization,
        source_root,
        identity,
        git_token,
        rpc_token,
        actor_id,
        release_id,
        release_agent_id,
        repository_id,
        instance_id,
        revision_id,
        attachment_id,
        installation_id,
        generation_id,
        session_id,
        initial_head,
        initial_receive_count,
        initial_record_blobs,
        previous_runtime_session_ids,
    } = state;
    eprintln!("HEPH_SESSION_CHAT_BROWSER stage=restart-started");
    run_session_chat_browser(
        database_url,
        running,
        project,
        release_agent_id,
        Uuid::nil(),
        SessionChatBrowserMode::Existing {
            repository_id,
            installation_id,
            generation_id,
            actor_id,
        },
    )
    .await;
    broker.wait_for_observed(3).await;
    let requests = broker.observed_snapshot();
    assert_eq!(
        requests.len(),
        3,
        "restart flow must make three model turns"
    );
    let first = &requests[0];
    let second = &requests[1];
    let third = &requests[2];
    assert_eq!(third.session_id, session_id);
    assert_eq!(third.messages.len(), 5);
    assert_eq!(third.messages[0].role, "user");
    assert_eq!(third.messages[1].role, "assistant");
    assert_eq!(third.messages[2].role, "user");
    assert_eq!(third.messages[3].role, "assistant");
    assert_eq!(third.messages[4].role, "user");
    assert_eq!(third.messages[0].record_id, first.record_id);
    assert_eq!(third.messages[2].record_id, second.record_id);
    assert_eq!(third.messages[4].record_id, third.record_id);
    assert_ne!(third.record_id, first.record_id);
    assert_ne!(third.record_id, second.record_id);

    let head = git_output_bare(root, repository_id, &["rev-parse", "refs/heads/main"]).await;
    assert_ne!(
        head, initial_head,
        "restart must persist a new canonical turn"
    );
    let ancestor = Command::new("git")
        .arg(format!(
            "--git-dir={}",
            root.join("repositories")
                .join(format!("{repository_id}.git"))
                .display()
        ))
        .args(["merge-base", "--is-ancestor", &initial_head, &head])
        .status()
        .await
        .expect("check restart Git ancestry");
    assert!(
        ancestor.success(),
        "restart head must retain the first two turns"
    );
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
        3,
        "restart must retain three human records"
    );
    assert_eq!(
        agent_paths.len(),
        3,
        "restart must retain three assistant records"
    );
    let mut human_record_paths = HashMap::with_capacity(human_paths.len());
    for path in &human_paths {
        let record: JsonValue = serde_json::from_str(
            &git_output_bare(root, repository_id, &["show", &format!("{head}:{path}")]).await,
        )
        .expect("restart human record JSON");
        let record_id = Uuid::parse_str(
            record["record_id"]
                .as_str()
                .expect("restart human record ID"),
        )
        .expect("restart human record UUID");
        assert_eq!(record["kind"], "user_message");
        human_record_paths.insert(record_id, path.clone());
    }
    let mut agent_record_paths = HashMap::with_capacity(agent_paths.len());
    for path in &agent_paths {
        let record: JsonValue = serde_json::from_str(
            &git_output_bare(root, repository_id, &["show", &format!("{head}:{path}")]).await,
        )
        .expect("restart assistant record JSON");
        let record_id = Uuid::parse_str(
            record["record_id"]
                .as_str()
                .expect("restart assistant record ID"),
        )
        .expect("restart assistant record UUID");
        assert_eq!(record["kind"], "assistant_message");
        assert_eq!(record["content"]["text"], MODEL_RESPONSE_TEXT);
        agent_record_paths.insert(record_id, record);
    }
    let first_agent_id = agent_record_paths
        .iter()
        .find(|(_, record)| record["in_reply_to"] == first.record_id.to_string())
        .map(|(record_id, _)| *record_id)
        .expect("restart first assistant record");
    let second_agent_id = agent_record_paths
        .iter()
        .find(|(_, record)| record["in_reply_to"] == second.record_id.to_string())
        .map(|(record_id, _)| *record_id)
        .expect("restart second assistant record");
    assert_eq!(third.messages[1].record_id, first_agent_id);
    assert_eq!(third.messages[3].record_id, second_agent_id);
    let third_agent_id = agent_record_paths
        .iter()
        .find(|(_, record)| record["in_reply_to"] == third.record_id.to_string())
        .map(|(record_id, _)| *record_id)
        .expect("restart third assistant record");
    let human_path = human_record_paths
        .get(&third.record_id)
        .expect("restart third human record path");
    let human_commit = canonical_record_commit(root, repository_id, human_path).await;
    assert_eq!(
        git_output_bare(
            root,
            repository_id,
            &["rev-parse", &format!("{human_commit}^")],
        )
        .await,
        initial_head,
        "third human commit must directly follow the pre-restart head"
    );
    let agent_path = agent_paths
        .iter()
        .find(|path| path.ends_with(&format!("{third_agent_id}.json")))
        .expect("restart third assistant record path");
    let agent_commit = canonical_record_commit(root, repository_id, agent_path).await;
    assert_eq!(
        git_output_bare(
            root,
            repository_id,
            &["rev-parse", &format!("{agent_commit}^")],
        )
        .await,
        human_commit,
        "restart assistant commit must be based on its human commit"
    );
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
    assert_eq!(accepted_runtime_receive_count(pool, run_id).await, 1);
    let expected_human_record_id = third.record_id.to_string();
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
    let (runtime_created_at, runtime_session_id): (OffsetDateTime, Uuid) = sqlx::query_as(
        "SELECT session.created_at, session.id
           FROM runtime_authority_sessions AS session
          WHERE session.run_id = $1",
    )
    .bind(run_id)
    .fetch_one(pool)
    .await
    .expect("restart runtime authority session");
    assert!(runtime_created_at > restart_boundary);
    assert!(!previous_runtime_session_ids.contains(&runtime_session_id));
    assert_ne!(runtime_session_id, Uuid::nil());
    for (path, expected) in &initial_record_blobs {
        assert_eq!(
            git_output_bare_bytes(root, repository_id, &["show", &format!("{head}:{path}")]).await,
            expected.as_slice(),
            "pre-restart record blob changed: {path}"
        );
    }
    let run_request_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM run_requests WHERE instance_id = $1 AND repository_id = $2",
    )
    .bind(instance_id)
    .bind(repository_id)
    .fetch_one(pool)
    .await
    .expect("restart session run request count");
    assert_eq!(
        run_request_count, 4,
        "restart must not recursively schedule runs"
    );
    let receive_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM git_receives WHERE repository_id = $1")
            .bind(repository_id)
            .fetch_one(pool)
            .await
            .expect("restart session Git receive count");
    assert_eq!(
        receive_count,
        initial_receive_count + 2,
        "restart must add exactly one human and one assistant receive"
    );
    let (current_installation_id, current_generation_id, current_release_id): (Uuid, Uuid, Uuid) =
        sqlx::query_as(
            "SELECT installation.id, installation.current_generation_id, generation.release_id
           FROM ui_installations installation
           JOIN ui_installation_generations generation
             ON generation.id = installation.current_generation_id
          WHERE installation.id = $1
            AND installation.project_id = $2
            AND installation.repository_id = $3
            AND installation.lifecycle = 'enabled'
            AND generation.release_id = $4",
        )
        .bind(installation_id)
        .bind(project.as_uuid())
        .bind(repository_id)
        .bind(release_id)
        .fetch_one(pool)
        .await
        .expect("restart installed UI persistence");
    assert_eq!(current_installation_id, installation_id);
    assert_eq!(current_generation_id, generation_id);
    assert_eq!(current_release_id, release_id);
    let current_revision_id: Uuid =
        sqlx::query_scalar("SELECT active_revision_id FROM agent_instances WHERE id = $1")
            .bind(instance_id)
            .fetch_one(pool)
            .await
            .expect("restart active instance revision");
    assert_eq!(current_revision_id, revision_id);
    eprintln!("HEPH_SESSION_CHAT_BROWSER stage=restart-canonical-validation-passed turns=3");
    finish_restart(
        pool,
        database_url,
        running,
        root,
        source_root,
        project,
        organization,
        identity,
        git_token,
        rpc_token,
        broker,
        actor_id,
        release_id,
        release_agent_id,
        repository_id,
        instance_id,
        revision_id,
        attachment_id,
        installation_id,
        generation_id,
        session_id,
    )
    .await;
}
