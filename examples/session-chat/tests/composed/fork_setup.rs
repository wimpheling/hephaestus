use super::fork;
use super::*;
use forge_domain::ProjectId;
use hephaestus_app::RunningHephaestus;
use identity_domain::{AuthenticatedIdentity, OrganizationId};
use sqlx::PgPool;
use std::path::Path;
use uuid::Uuid;
#[allow(clippy::too_many_arguments)]
// Fork setup passes independent production fixture handles into one phase.
pub(crate) async fn prepare_fork(
    pool: &PgPool,
    running: &RunningHephaestus,
    root: &Path,
    source_root: &Path,
    project: ProjectId,
    organization: OrganizationId,
    identity: &AuthenticatedIdentity,
    git_token: &str,
    rpc_token: &(dyn Fn(&str) -> String + Send + Sync),
    source: fork::SourceSessionState,
) -> (
    fork::SourceSessionState,
    fork::ForkTargetState,
    fork::ForkTargetProvisioningState,
) {
    let source_before = source.clone();
    let target = fork::exercise(
        pool,
        running,
        root,
        source_root,
        project,
        source,
        git_token,
        rpc_token,
    )
    .await;
    assert_eq!(target.source_repository_id, source_before.repository_id);
    assert_eq!(target.source_head, source_before.head);
    assert_eq!(
        target.source_model_binding_id,
        source_before.model_binding_id
    );
    assert_eq!(
        target.source_accepted_receive_count,
        source_before.accepted_receive_count
    );
    assert!(
        target.checkout.is_dir(),
        "fork checkout must remain materialized"
    );
    assert!(!target.manifest_commit.is_empty());
    let target_rule_id = Uuid::new_v4();
    let provisioned = fork::provision_target(
        pool,
        running,
        project,
        organization,
        identity,
        rpc_token,
        &target,
        target.source_release_id,
        target.source_release_agent_id,
        target_rule_id,
    )
    .await;
    assert_ne!(provisioned.revision_id, source_before.revision_id);
    assert_ne!(
        provisioned.capability_revision_id,
        source_before.revision_id
    );
    assert_ne!(provisioned.bound_revision_id, source_before.revision_id);
    assert_ne!(provisioned.binding_id, source_before.model_binding_id);
    let binding_revision_id: Uuid =
        sqlx::query_scalar("SELECT instance_revision_id FROM agent_secret_bindings WHERE id = $1")
            .bind(provisioned.binding_id)
            .fetch_one(pool)
            .await
            .expect("fork target fresh model binding");
    assert_eq!(binding_revision_id, provisioned.bound_revision_id);
    eprintln!("HEPH_SESSION_CHAT_BROWSER stage=fork-started");
    (source_before, target, provisioned)
}
