//! Navigation fixture orchestration.

use forge_domain::{OrganizationId, ProjectId, RepositoryId};
use identity_domain::UserId;
use release_domain::{ReleaseId, UiInstallationState, UiInstallationTarget};
use sqlx::PgPool;

use super::seed_rows::{
    install_row, seed_owner_and_target, seed_published_release, seed_source_target,
    seed_ui_descriptor,
};

pub(super) struct Seeded {
    pub(super) actor: UserId,
    pub(super) organization: OrganizationId,
    pub(super) project: ProjectId,
    pub(super) repository: RepositoryId,
    pub(super) source_project: ProjectId,
    pub(super) second_organization: OrganizationId,
    pub(super) release: ReleaseId,
}

pub(super) async fn seed_all(pool: &PgPool) -> Seeded {
    let actor = UserId::new();
    let organization = OrganizationId::new();
    let project = ProjectId::new();
    let repository = RepositoryId::new();
    let source_project = ProjectId::new();
    let source_repository = RepositoryId::new();
    let second_organization = OrganizationId::new();
    let second_project = ProjectId::new();
    let second_repository = RepositoryId::new();
    seed_owner_and_target(pool, actor, organization, project, repository).await;
    seed_source_target(pool, actor, organization, source_project, source_repository).await;
    seed_owner_and_target(
        pool,
        actor,
        second_organization,
        second_project,
        second_repository,
    )
    .await;
    let release = seed_published_release(pool, actor, source_repository, 1).await;
    let second_release = seed_published_release(pool, actor, second_repository, 2).await;
    seed_ui_descriptor(pool, release, "project-ui", "project").await;
    seed_ui_descriptor(pool, release, "project-next", "project").await;
    seed_ui_descriptor(pool, release, "project-live", "project").await;
    seed_ui_descriptor(pool, release, "repository-ui", "repository").await;
    seed_ui_descriptor(pool, release, "global-ui", "global").await;
    seed_ui_descriptor(pool, second_release, "global-ui", "global").await;
    install_row(
        pool,
        actor,
        UiInstallationTarget::project(project),
        "project-ui",
        UiInstallationState::Removed,
        release,
    )
    .await;
    install_row(
        pool,
        actor,
        UiInstallationTarget::project(project),
        "project-live",
        UiInstallationState::Enabled,
        release,
    )
    .await;
    install_row(
        pool,
        actor,
        UiInstallationTarget::project(project),
        "project-next",
        UiInstallationState::Enabled,
        release,
    )
    .await;
    install_row(
        pool,
        actor,
        UiInstallationTarget::repository(repository),
        "repository-ui",
        UiInstallationState::Enabled,
        release,
    )
    .await;
    install_row(
        pool,
        actor,
        UiInstallationTarget::organization(organization),
        "global-ui",
        UiInstallationState::Disabled,
        release,
    )
    .await;
    install_row(
        pool,
        actor,
        UiInstallationTarget::organization(second_organization),
        "global-ui",
        UiInstallationState::Enabled,
        second_release,
    )
    .await;
    for release_id in [release, second_release] {
        sqlx::query("UPDATE releases SET state = 'published', publication_actor_id = $2, published_at = now() WHERE id = $1")
            .bind(release_id.as_uuid())
            .bind(actor.as_uuid())
            .execute(pool)
            .await
            .expect("publish source release after immutable fixture inserts");
    }
    Seeded {
        actor,
        organization,
        project,
        repository,
        source_project,
        second_organization,
        release,
    }
}
