use super::*;
use crate::{
    fixture::{FixtureSeedIds, insert_canonical_session},
    fixture_bindings::seed_installation_bindings,
    fixture_gateway::seed_managed_gateway,
    fixture_identity::seed_identity_source,
    fixture_installations::{
        seed_global_installation, seed_project_installation, seed_repository_installation,
    },
    fixture_release::seed_release_descriptors,
};

/// Build the shared matrix fixture in publication-parent order so every
/// negative case reaches the intended composite constraint.
pub async fn seed_fixture_reusing_installation_helpers(worker: &PgPool) -> Fixture {
    let ids = FixtureSeedIds::new();
    seed_identity_source(worker, &ids).await;
    seed_release_descriptors(worker, &ids).await;
    seed_managed_gateway(worker, &ids).await;
    insert_canonical_session(worker, ids.parent_session, ids.actor, ids.request_id, 20).await;
    insert_canonical_session(
        worker,
        ids.outsider_parent_session,
        ids.outsider,
        Uuid::new_v4(),
        20,
    )
    .await;
    seed_project_installation(
        worker,
        ids.installation,
        ids.generation,
        ids.release,
        "schema-ui",
        ids.actor,
        ids.project,
    )
    .await;
    seed_global_installation(
        worker,
        ids.global_installation,
        ids.global_generation,
        ids.release,
        "schema-global",
        ids.actor,
        ids.organization,
    )
    .await;
    for (installation, generation, key) in [
        (
            ids.repository_installation,
            ids.repository_generation,
            "schema-repository",
        ),
        (
            ids.no_git_repository_installation,
            ids.no_git_repository_generation,
            "schema-repository-no-git",
        ),
        (
            ids.write_repository_installation,
            ids.write_repository_generation,
            "schema-repository-write",
        ),
    ] {
        seed_repository_installation(
            worker,
            installation,
            generation,
            ids.release,
            key,
            ids.actor,
            ids.source_project,
            ids.repository,
        )
        .await;
    }
    seed_project_installation(
        worker,
        ids.other_installation,
        ids.other_generation,
        ids.release,
        "schema-ui-two",
        ids.actor,
        ids.project,
    )
    .await;
    seed_project_installation(
        worker,
        ids.managed_installation,
        ids.managed_generation,
        ids.release,
        "schema-managed",
        ids.actor,
        ids.project,
    )
    .await;
    seed_installation_bindings(worker, &ids).await;
    ids.fixture()
}
