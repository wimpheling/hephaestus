use capability_domain::{AuthorityHash, CapabilityBindingId, CapabilityRequirement};
use release_domain::{AgentInstanceId, AgentInstanceRevisionId};

pub struct Prepared {
    pub(super) instance_id: AgentInstanceId,
    pub(super) initial_revision_id: AgentInstanceRevisionId,
    pub(super) source_binding_id: CapabilityBindingId,
    pub(super) requirement: CapabilityRequirement,
    pub(super) source_binding_hash: AuthorityHash,
    pub(super) new_revision_id: AgentInstanceRevisionId,
}

mod assertions;
mod fixture;

#[tokio::test]
#[serial_test::serial]
async fn bind_secret_carries_runtime_git_authority_to_new_revision() {
    let Some(pool) = crate::support::pool().await else {
        return;
    };
    sqlx::migrate!("../../../../migrations")
        .run(&pool)
        .await
        .expect("apply application migrations");
    let fixture = crate::support::seed(&pool).await;
    let (_instance_id, _revision_id, _attachment_id, _) =
        crate::support::seed_instance(&pool, &fixture).await;
    let prepared = fixture::prepare(&pool, &fixture).await;
    assertions::assert_carried(&pool, &fixture, &prepared).await;
}
