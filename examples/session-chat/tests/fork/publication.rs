use std::path::Path;

use forge_domain::ProjectId;
use hephaestus_app::RunningHephaestus;
use serde_json::Value as JsonValue;
use sqlx::PgPool;
use uuid::Uuid;

use super::{
    helpers::{
        authenticated_git_pat, commit_ids, create_target_repository, fork_local_checkout,
        issue_target_pat, object_ids,
    },
    state::{ForkTargetState, SourceSessionState},
};

/// Publishes a fork through the production Git HTTP boundary and proves that
/// the source history is retained without mutating the source repository.
#[allow(
    clippy::cognitive_complexity,
    clippy::too_many_arguments,
    clippy::too_many_lines
)] // This test keeps the publication proof in one auditable production boundary.
pub async fn exercise(
    pool: &PgPool,
    running: &RunningHephaestus,
    root: &Path,
    source_root: &Path,
    project: ProjectId,
    source: SourceSessionState,
    git_token: &str,
    rpc_token: &(dyn Fn(&str) -> String + Send + Sync),
) -> ForkTargetState {
    let actual_source_head = super::super::git_output_bare(
        root,
        source.repository_id,
        &["rev-parse", "refs/heads/main"],
    )
    .await;
    assert_eq!(
        actual_source_head, source.head,
        "source snapshot head must match the live source ref before forking"
    );
    let actual_source_objects = object_ids(root, source.repository_id, &source.head).await;
    assert!(
        !actual_source_objects.is_empty(),
        "source snapshot must contain reachable objects"
    );
    assert_eq!(
        actual_source_objects, source.reachable_objects,
        "source snapshot must still describe the live source history"
    );
    assert!(
        !commit_ids(root, source.repository_id, &source.head)
            .await
            .is_empty(),
        "source snapshot must contain reachable commits"
    );
    assert_eq!(
        super::super::accepted_receive_count(pool, source.repository_id).await,
        source.accepted_receive_count,
        "source receive snapshot must still be current before forking"
    );
    let target_repository_id = create_target_repository(running, rpc_token, project).await;
    let target_pat = issue_target_pat(running, rpc_token, target_repository_id).await;

    let source_clone = root.join(format!("session-chat-fork-source-{}", Uuid::new_v4()));
    let target_checkout = root.join(format!("session-chat-fork-target-{}", Uuid::new_v4()));
    let source_remote = format!("http://{}/{}", running.http_addr(), source.repository_id);
    let source_clone_arg = source_clone.to_string_lossy().into_owned();
    let clone_arguments = [
        "clone",
        "--branch",
        "main",
        source_remote.as_str(),
        source_clone_arg.as_str(),
    ];
    super::super::authenticated_git(root, git_token, &clone_arguments).await;
    assert_eq!(
        super::super::git_output(&source_clone, &["rev-parse", "refs/heads/main"]).await,
        source.head,
        "HTTP source clone must start at the captured source head"
    );

    let target_session_id = Uuid::new_v4();
    fork_local_checkout(
        source_root,
        &source_clone,
        &target_checkout,
        target_session_id,
    )
    .await;
    let target_remote = format!("http://{}/{}", running.http_addr(), target_repository_id);
    let add_remote_arguments = ["remote", "add", "origin", target_remote.as_str()];
    super::super::git(&target_checkout, &add_remote_arguments).await;
    let push_arguments = ["push", "origin", "HEAD:refs/heads/main"];
    authenticated_git_pat(&target_checkout, &target_pat, &push_arguments).await;

    let manifest_commit = super::super::git_output_bare(
        root,
        target_repository_id,
        &["rev-parse", "refs/heads/main"],
    )
    .await;
    let source_commit_ids = commit_ids(root, source.repository_id, &source.head).await;
    let target_commit_ids = commit_ids(root, target_repository_id, &manifest_commit).await;
    assert!(
        source_commit_ids.is_subset(&target_commit_ids),
        "fork target must contain every source reachable commit"
    );
    assert_eq!(
        target_commit_ids.difference(&source_commit_ids).count(),
        1,
        "fork publication must add exactly one commit before the target turn"
    );
    assert_eq!(
        super::super::git_output_bare(
            root,
            target_repository_id,
            &["rev-parse", &format!("{manifest_commit}^")],
        )
        .await,
        source.head,
        "fork manifest must directly descend from the source head"
    );

    let target_objects = object_ids(root, target_repository_id, &manifest_commit).await;
    assert!(
        source
            .reachable_objects
            .iter()
            .all(|object| target_objects.contains(object)),
        "fork target must retain every source reachable object"
    );
    for (path, expected) in &source.record_blobs {
        assert_eq!(
            super::super::git_output_bare_bytes(
                root,
                target_repository_id,
                &["show", &format!("{manifest_commit}:{path}")],
            )
            .await,
            expected.as_slice(),
            "fork target must retain the source record bytes: {path}"
        );
    }
    let manifest_paths = super::super::git_output_bare(
        root,
        target_repository_id,
        &[
            "diff-tree",
            "--no-commit-id",
            "--name-only",
            "-r",
            &manifest_commit,
        ],
    )
    .await
    .lines()
    .map(str::to_owned)
    .collect::<Vec<_>>();
    assert_eq!(
        manifest_paths.len(),
        1,
        "fork commit must contain only its new session manifest"
    );
    let manifest_path = manifest_paths
        .into_iter()
        .next()
        .expect("fork manifest path");
    assert_eq!(
        manifest_path, ".heph/session/v1/manifest.json",
        "fork commit must update the release-owned session manifest"
    );
    let manifest: JsonValue = serde_json::from_str(
        &super::super::git_output_bare(
            root,
            target_repository_id,
            &["show", &format!("{manifest_commit}:{manifest_path}")],
        )
        .await,
    )
    .expect("fork manifest JSON");
    assert_eq!(manifest["kind"], "session_manifest");
    assert_eq!(
        manifest["data"]["session_id"],
        target_session_id.to_string()
    );
    assert_eq!(
        manifest["data"]["forked_from_session_id"],
        source.session_id.to_string()
    );

    assert_eq!(
        super::super::git_output(&target_checkout, &["remote", "get-url", "origin"]).await,
        target_remote,
        "fork checkout must point only at the target repository"
    );
    assert_eq!(
        super::super::git_output_bare(
            root,
            source.repository_id,
            &["rev-parse", "refs/heads/main"]
        )
        .await,
        source.head,
        "fork must preserve the source main ref"
    );
    assert_eq!(
        super::super::accepted_receive_count(pool, source.repository_id).await,
        source.accepted_receive_count,
        "fork must not add a source receive"
    );

    ForkTargetState {
        source_repository_id: source.repository_id,
        source_head: source.head,
        source_accepted_receive_count: source.accepted_receive_count,
        source_release_id: source.release_id,
        source_release_agent_id: source.release_agent_id,
        source_model_rule_id: source.model_rule_id,
        source_model_binding_id: source.model_binding_id,
        source_instance_id: source.instance_id,
        source_revision_id: source.revision_id,
        source_attachment_id: source.attachment_id,
        source_installation_id: source.installation_id,
        source_generation_id: source.generation_id,
        repository_id: target_repository_id,
        checkout: target_checkout,
        session_id: target_session_id,
        manifest_commit,
        manifest_path,
        initial_accepted_receive_count: super::super::accepted_receive_count(
            pool,
            target_repository_id,
        )
        .await,
    }
}
