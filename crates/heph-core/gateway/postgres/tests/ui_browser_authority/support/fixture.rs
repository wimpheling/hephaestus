//! Shared release, gateway, and installation fixture orchestration.

use super::{installations::seed_installations, release::seed_release};
use sqlx::PgPool;
use uuid::Uuid;

#[allow(dead_code)]
pub struct Fixture {
    pub actor: Uuid,
    pub outsider: Uuid,
    pub organization: Uuid,
    pub project: Uuid,
    pub source_project: Uuid,
    pub release: Uuid,
    pub release_agent: Uuid,
    pub other_organization: Uuid,
    pub parent_session: Uuid,
    pub outsider_parent_session: Uuid,
    pub installation: Uuid,
    pub other_installation: Uuid,
    pub generation: Uuid,
    pub other_generation: Uuid,
    pub global_installation: Uuid,
    pub global_generation: Uuid,
    pub repository_installation: Uuid,
    pub repository_generation: Uuid,
    pub managed_installation: Uuid,
    pub managed_generation: Uuid,
    pub managed_gateway: Uuid,
    pub managed_revision: Uuid,
    pub route: &'static str,
}

#[derive(Clone, Copy)]
pub struct SeedIds {
    pub actor: Uuid,
    pub outsider: Uuid,
    pub organization: Uuid,
    pub other_organization: Uuid,
    pub project: Uuid,
    pub source_project: Uuid,
    pub repository: Uuid,
    pub receive: Uuid,
    pub build: Uuid,
    pub source_revision: Uuid,
    pub release: Uuid,
    pub artifact: Uuid,
    pub parent_session: Uuid,
    pub outsider_parent_session: Uuid,
    pub installation: Uuid,
    pub other_installation: Uuid,
    pub generation: Uuid,
    pub other_generation: Uuid,
    pub global_installation: Uuid,
    pub global_generation: Uuid,
    pub repository_installation: Uuid,
    pub repository_generation: Uuid,
    pub managed_installation: Uuid,
    pub managed_generation: Uuid,
    pub managed_gateway: Uuid,
    pub managed_revision: Uuid,
    pub release_agent: Uuid,
    pub agent_family: Uuid,
    pub request_id: Uuid,
}

impl SeedIds {
    pub fn new() -> Self {
        Self {
            actor: Uuid::new_v4(),
            outsider: Uuid::new_v4(),
            organization: Uuid::new_v4(),
            other_organization: Uuid::new_v4(),
            project: Uuid::new_v4(),
            source_project: Uuid::new_v4(),
            repository: Uuid::new_v4(),
            receive: Uuid::new_v4(),
            build: Uuid::new_v4(),
            source_revision: Uuid::new_v4(),
            release: Uuid::new_v4(),
            artifact: Uuid::new_v4(),
            parent_session: Uuid::new_v4(),
            outsider_parent_session: Uuid::new_v4(),
            installation: Uuid::new_v4(),
            other_installation: Uuid::new_v4(),
            generation: Uuid::new_v4(),
            other_generation: Uuid::new_v4(),
            global_installation: Uuid::new_v4(),
            global_generation: Uuid::new_v4(),
            repository_installation: Uuid::new_v4(),
            repository_generation: Uuid::new_v4(),
            managed_installation: Uuid::new_v4(),
            managed_generation: Uuid::new_v4(),
            managed_gateway: Uuid::new_v4(),
            managed_revision: Uuid::new_v4(),
            release_agent: Uuid::new_v4(),
            agent_family: Uuid::new_v4(),
            request_id: Uuid::new_v4(),
        }
    }
}

pub async fn seed_fixture_reusing_installation_helpers(worker: &PgPool) -> Fixture {
    let ids = SeedIds::new();
    seed_release(worker, &ids).await;
    seed_installations(worker, &ids).await;
    Fixture {
        actor: ids.actor,
        outsider: ids.outsider,
        organization: ids.organization,
        project: ids.project,
        source_project: ids.source_project,
        release: ids.release,
        release_agent: ids.release_agent,
        other_organization: ids.other_organization,
        parent_session: ids.parent_session,
        outsider_parent_session: ids.outsider_parent_session,
        installation: ids.installation,
        other_installation: ids.other_installation,
        generation: ids.generation,
        other_generation: ids.other_generation,
        global_installation: ids.global_installation,
        global_generation: ids.global_generation,
        repository_installation: ids.repository_installation,
        repository_generation: ids.repository_generation,
        managed_installation: ids.managed_installation,
        managed_generation: ids.managed_generation,
        managed_gateway: ids.managed_gateway,
        managed_revision: ids.managed_revision,
        route: "schema-ui",
    }
}
