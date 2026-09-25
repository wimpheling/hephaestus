use super::denial::accepted_receive_count;
use super::fork;
use super::fork_setup::prepare_fork;
use super::git_queries::{accepted_runtime_receive_count, canonical_record_commit};
use super::rpc_helpers::{git_output_bare, git_output_bare_bytes};
use super::runtime_assertions::assert_runtime_git_turn_at_commit;
use super::*;
use super::{MODEL_RESPONSE_TEXT, SessionBrokerFixture};
use forge_domain::ProjectId;
use hephaestus_app::RunningHephaestus;
use identity_domain::{AuthenticatedIdentity, OrganizationId};
use serde_json::Value as JsonValue;
use sqlx::PgPool;
use std::{collections::HashMap, path::Path, time::Duration};
use uuid::Uuid;

/// Publishes and exercises the forked repository while retaining the source
/// broker observer.  The caller owns the restart boundary and supplies the
/// same production identity/token factories used by the source phase.
#[allow(
    clippy::cognitive_complexity,
    clippy::too_many_arguments,
    clippy::too_many_lines
)] // Fork host assertions intentionally cover Git, browser, and runtime provenance together.
pub(crate) async fn exercise_browser_fork(
    pool: &PgPool,
    database_url: &str,
    running: &RunningHephaestus,
    root: &Path,
    source_root: &Path,
    project: ProjectId,
    organization: OrganizationId,
    identity: &AuthenticatedIdentity,
    git_token: &str,
    rpc_token: &(dyn Fn(&str) -> String + Send + Sync),
    broker: SessionBrokerFixture,
    source: fork::SourceSessionState,
) {
    let (source_before, target, provisioned) = prepare_fork(
        pool,
        running,
        root,
        source_root,
        project,
        organization,
        identity,
        git_token,
        rpc_token,
        source,
    )
    .await;
    run_session_chat_browser(
        database_url,
        running,
        project,
        target.source_release_agent_id,
        Uuid::nil(),
        SessionChatBrowserMode::Fork {
            repository_id: target.repository_id,
            installation_id: provisioned.installation_id,
            generation_id: provisioned.generation_id,
            actor_id: identity.user_id.as_uuid(),
            initial_transcript_count: 10,
            initial_agent_count: 5,
        },
    )
    .await;
    broker.wait_for_observed(6).await;
    let requests = broker.observed_snapshot();
    assert_eq!(requests.len(), 6, "fork flow must make six model turns");
    let target_request = &requests[5];
    assert_ne!(target_request.session_id, source_before.session_id);
    assert_eq!(target_request.session_id, target.session_id);
    assert_eq!(target_request.messages.len(), 11);
    for (index, message) in target_request.messages.iter().enumerate() {
        assert_eq!(
            message.role,
            if index % 2 == 0 { "user" } else { "assistant" }
        );
    }
    assert_eq!(
        target_request
            .messages
            .last()
            .map(|message| message.record_id),
        Some(target_request.record_id)
    );

    let target_head = git_output_bare(
        root,
        target.repository_id,
        &["rev-parse", "refs/heads/main"],
    )
    .await;
    let source_commit_count = git_output_bare(
        root,
        source_before.repository_id,
        &["rev-list", &source_before.head],
    )
    .await
    .lines()
    .count();
    let target_commit_count =
        git_output_bare(root, target.repository_id, &["rev-list", &target_head])
            .await
            .lines()
            .count();
    assert_eq!(
        target_commit_count,
        source_commit_count + 3,
        "target history must contain the source, manifest, human, and assistant commits only"
    );
    let target_paths = git_output_bare(
        root,
        target.repository_id,
        &["ls-tree", "-r", "--name-only", &target_head],
    )
    .await
    .lines()
    .map(str::to_owned)
    .collect::<Vec<_>>();
    let human_paths = target_paths
        .iter()
        .filter(|path| path.starts_with(".heph/session/v1/records/human/"))
        .cloned()
        .collect::<Vec<_>>();
    let agent_paths = target_paths
        .iter()
        .filter(|path| path.starts_with(".heph/session/v1/records/agent/"))
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(
        human_paths.len(),
        6,
        "fork target must retain six human records"
    );
    assert_eq!(
        agent_paths.len(),
        6,
        "fork target must retain six assistant records"
    );
    for (path, expected) in &source_before.record_blobs {
        assert_eq!(
            git_output_bare_bytes(
                root,
                target.repository_id,
                &["show", &format!("{target_head}:{path}")],
            )
            .await,
            expected.as_slice(),
            "fork target source record changed after target turn: {path}"
        );
    }
    let target_manifest: JsonValue = serde_json::from_str(
        &git_output_bare(
            root,
            target.repository_id,
            &["show", &format!("{target_head}:{}", target.manifest_path)],
        )
        .await,
    )
    .expect("fork target manifest JSON");
    assert_eq!(
        target_manifest["data"]["session_id"],
        target.session_id.to_string()
    );
    assert_eq!(
        target_manifest["data"]["forked_from_session_id"],
        source_before.session_id.to_string()
    );

    let mut human_record_paths = HashMap::new();
    for path in &human_paths {
        let record: JsonValue = serde_json::from_str(
            &git_output_bare(
                root,
                target.repository_id,
                &["show", &format!("{target_head}:{path}")],
            )
            .await,
        )
        .expect("fork target human record JSON");
        let record_id = Uuid::parse_str(
            record["record_id"]
                .as_str()
                .expect("fork target human record ID"),
        )
        .expect("fork target human record UUID");
        human_record_paths.insert(record_id, path.clone());
    }
    let mut agent_record_paths = HashMap::new();
    for path in &agent_paths {
        let record: JsonValue = serde_json::from_str(
            &git_output_bare(
                root,
                target.repository_id,
                &["show", &format!("{target_head}:{path}")],
            )
            .await,
        )
        .expect("fork target assistant record JSON");
        let record_id = Uuid::parse_str(
            record["record_id"]
                .as_str()
                .expect("fork target assistant record ID"),
        )
        .expect("fork target assistant record UUID");
        assert_eq!(record["content"]["text"], MODEL_RESPONSE_TEXT);
        agent_record_paths.insert(record_id, (path.clone(), record));
    }
    let human_path = human_record_paths
        .get(&target_request.record_id)
        .expect("fork target request human record");
    let human_commit = canonical_record_commit(root, target.repository_id, human_path).await;
    let agent_entry = agent_record_paths
        .values()
        .find(|(_, record)| record["in_reply_to"] == target_request.record_id.to_string())
        .expect("fork target assistant response");
    let agent_commit = canonical_record_commit(root, target.repository_id, &agent_entry.0).await;
    assert_eq!(
        git_output_bare(
            root,
            target.repository_id,
            &["rev-parse", &format!("{human_commit}^")],
        )
        .await,
        target.manifest_commit,
        "target human commit must directly follow the fork manifest"
    );
    assert_eq!(
        git_output_bare(
            root,
            target.repository_id,
            &["rev-parse", &format!("{agent_commit}^")],
        )
        .await,
        human_commit,
        "fork target assistant commit must directly follow its human commit"
    );
    let run_id = accepted_normal_run_id(
        pool,
        target.repository_id,
        provisioned.instance_id,
        provisioned.attachment_id,
        identity.user_id.as_uuid(),
        &human_commit,
    )
    .await;
    wait_for_run_succeeded(pool, run_id, Duration::from_secs(120)).await;
    let expected_human_record_id = target_request.record_id.to_string();
    assert_runtime_git_turn_at_commit(
        pool,
        root,
        target.repository_id,
        provisioned.instance_id,
        provisioned.attachment_id,
        identity.user_id.as_uuid(),
        run_id,
        &human_commit,
        &agent_commit,
        Some(&expected_human_record_id),
    )
    .await;
    let (runtime_revision_id, authority_revision_id, runtime_instance_id, runtime_attachment_id): (
        Uuid,
        Uuid,
        Uuid,
        Uuid,
    ) = sqlx::query_as(
        "SELECT run.instance_revision_id, session.instance_revision_id,
                    session.instance_id, session.attachment_id
               FROM runs AS run
               JOIN runtime_authority_sessions AS session ON session.run_id = run.id
              WHERE run.id = $1",
    )
    .bind(run_id)
    .fetch_one(pool)
    .await
    .expect("fork target runtime authority provenance");
    assert_eq!(runtime_revision_id, provisioned.bound_revision_id);
    assert_eq!(authority_revision_id, provisioned.bound_revision_id);
    assert_eq!(runtime_instance_id, provisioned.instance_id);
    assert_eq!(runtime_attachment_id, provisioned.attachment_id);
    assert_eq!(accepted_runtime_receive_count(pool, run_id).await, 1);
    let target_receive_count = accepted_receive_count(pool, target.repository_id).await;
    assert_eq!(
        target_receive_count,
        target.initial_accepted_receive_count + 2,
        "target turn must add exactly one human and one runtime receive"
    );
    let target_run_request_count: i64 = sqlx::query_scalar(
        "SELECT count(*)
           FROM run_requests
          WHERE instance_id = $1 AND repository_id = $2",
    )
    .bind(provisioned.instance_id)
    .bind(target.repository_id)
    .fetch_one(pool)
    .await
    .expect("fork target run request count");
    assert_eq!(
        target_run_request_count, 1,
        "fork target must schedule only its one new model turn"
    );
    assert_eq!(
        git_output_bare(
            root,
            target.repository_id,
            &["rev-parse", "refs/heads/main"]
        )
        .await,
        target_head
    );
    assert_eq!(
        target_head, agent_commit,
        "target main must finish at the assistant commit"
    );
    assert_eq!(
        git_output_bare(
            root,
            source_before.repository_id,
            &["rev-parse", "refs/heads/main"]
        )
        .await,
        source_before.head
    );
    assert_eq!(
        accepted_receive_count(pool, source_before.repository_id).await,
        source_before.accepted_receive_count
    );
    eprintln!("HEPH_SESSION_CHAT_BROWSER stage=fork-canonical-validation-passed target_turns=6");
    let _ = broker.assert_observed().await;
}

// Keep the concurrency assertions together so each production boundary is checked once.
