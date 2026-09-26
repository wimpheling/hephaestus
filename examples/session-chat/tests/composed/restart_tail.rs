use super::browser_concurrency::exercise_browser_concurrency;
use super::denial::accepted_receive_count;
use super::fork;
use super::fork_phase::exercise_browser_fork;
use super::rpc_helpers::{git_output_bare, git_output_bare_bytes};
use super::{SessionBrokerFixture, concurrent_e2e_enabled, fork_e2e_enabled};
use forge_domain::{OrganizationId, ProjectId};
use hephaestus_app::RunningHephaestus;
use identity_domain::AuthenticatedIdentity;
use sqlx::PgPool;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};
use uuid::Uuid;
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
// Restart tail validates one complete persisted browser session transition.
pub(crate) async fn finish_restart(
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
    actor_id: Uuid,
    release_id: Uuid,
    release_agent_id: Uuid,
    repository_id: Uuid,
    instance_id: Uuid,
    revision_id: Uuid,
    attachment_id: Uuid,
    installation_id: Uuid,
    generation_id: Uuid,
    session_id: Uuid,
) {
    if concurrent_e2e_enabled() {
        let broker = exercise_browser_concurrency(
            pool,
            database_url,
            running,
            root,
            broker,
            project,
            actor_id,
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
        if fork_e2e_enabled() {
            let source_head =
                git_output_bare(root, repository_id, &["rev-parse", "refs/heads/main"]).await;
            let reachable_objects = git_output_bare(
                root,
                repository_id,
                &["rev-list", "--objects", &source_head],
            )
            .await
            .lines()
            .filter_map(|line| line.split_whitespace().next().map(str::to_owned))
            .collect::<BTreeSet<_>>();
            assert!(!reachable_objects.is_empty());
            let record_paths = git_output_bare(
                root,
                repository_id,
                &["ls-tree", "-r", "--name-only", &source_head],
            )
            .await
            .lines()
            .filter(|path| path.starts_with(".heph/session/v1/records/"))
            .map(str::to_owned)
            .collect::<Vec<_>>();
            let mut record_blobs = BTreeMap::new();
            for path in record_paths {
                record_blobs.insert(
                    path.clone(),
                    git_output_bare_bytes(
                        root,
                        repository_id,
                        &["show", &format!("{source_head}:{path}")],
                    )
                    .await,
                );
            }
            assert_eq!(record_blobs.len(), 10);
            let (model_rule_id, model_binding_id): (Uuid, Uuid) = sqlx::query_as(
                "SELECT rule.id, rule.binding_id
                   FROM brokered_secret_rules AS rule
                   JOIN agent_secret_bindings AS binding
                     ON binding.id = rule.binding_id
                    AND binding.instance_revision_id = rule.instance_revision_id
                  WHERE rule.instance_revision_id = $1
                    AND binding.slot_key = 'model'
                    AND binding.status = 'active'
                    AND rule.destination_origin = 'https://api.model.example'",
            )
            .bind(revision_id)
            .fetch_one(pool)
            .await
            .expect("source session model rule and binding");
            let source = fork::SourceSessionState {
                repository_id,
                session_id,
                release_id,
                release_agent_id,
                model_rule_id,
                instance_id,
                revision_id,
                attachment_id,
                installation_id,
                generation_id,
                model_binding_id,
                head: source_head,
                reachable_objects,
                record_blobs,
                accepted_receive_count: accepted_receive_count(pool, repository_id).await,
            };
            exercise_browser_fork(
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
                source,
            )
            .await;
            return;
        }
        let final_requests = broker.assert_observed().await;
        assert_eq!(
            final_requests.len(),
            5,
            "concurrency flow must make five model turns"
        );
    } else {
        let final_requests = broker.assert_observed().await;
        assert_eq!(
            final_requests.len(),
            3,
            "restart flow must make three model turns"
        );
    }
}
