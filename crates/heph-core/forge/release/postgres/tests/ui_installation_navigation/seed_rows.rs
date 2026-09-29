//! SQL fixture row helpers for UI installation navigation.

use forge_domain::{OrganizationId, ProjectId, RepositoryId};
use identity_domain::UserId;
use release_domain::{
    ReleaseId, UiInstallationGenerationId, UiInstallationId, UiInstallationState,
    UiInstallationTarget,
};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

pub async fn seed_owner_and_target(
    pool: &PgPool,
    actor: UserId,
    organization: OrganizationId,
    project: ProjectId,
    repository: RepositoryId,
) {
    sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, 'ui-navigation-owner') ON CONFLICT (id) DO NOTHING")
        .bind(actor.as_uuid())
        .execute(pool)
        .await
        .expect("seed actor");
    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, 'ui-navigation-org')")
        .bind(organization.as_uuid())
        .execute(pool)
        .await
        .expect("seed organization");
    sqlx::query("INSERT INTO organization_members (organization_id, user_id, role) VALUES ($1, $2, 'owner')")
        .bind(organization.as_uuid())
        .bind(actor.as_uuid())
        .execute(pool)
        .await
        .expect("seed organization owner");
    sqlx::query(
        "INSERT INTO projects (id, organization_id, name) VALUES ($1, $2, 'ui-navigation-project')",
    )
    .bind(project.as_uuid())
    .bind(organization.as_uuid())
    .execute(pool)
    .await
    .expect("seed project");
    sqlx::query("INSERT INTO repositories (id, project_id, name) VALUES ($1, $2, 'ui-navigation-repository')")
        .bind(repository.as_uuid())
        .bind(project.as_uuid())
        .execute(pool)
        .await
        .expect("seed repository");
    sqlx::query("INSERT INTO project_maintainers (project_id, user_id) VALUES ($1, $2)")
        .bind(project.as_uuid())
        .bind(actor.as_uuid())
        .execute(pool)
        .await
        .expect("seed source release use permission");
}

pub async fn seed_source_target(
    pool: &PgPool,
    actor: UserId,
    organization: OrganizationId,
    project: ProjectId,
    repository: RepositoryId,
) {
    sqlx::query(
        "INSERT INTO projects (id, organization_id, name)
         VALUES ($1, $2, 'ui-navigation-source-project')",
    )
    .bind(project.as_uuid())
    .bind(organization.as_uuid())
    .execute(pool)
    .await
    .expect("seed source project");
    sqlx::query(
        "INSERT INTO repositories (id, project_id, name)
         VALUES ($1, $2, 'ui-navigation-source-repository')",
    )
    .bind(repository.as_uuid())
    .bind(project.as_uuid())
    .execute(pool)
    .await
    .expect("seed source repository");
    sqlx::query("INSERT INTO project_maintainers (project_id, user_id) VALUES ($1, $2)")
        .bind(project.as_uuid())
        .bind(actor.as_uuid())
        .execute(pool)
        .await
        .expect("seed source project read permission");
}

pub async fn seed_published_release(
    pool: &PgPool,
    actor: UserId,
    repository: RepositoryId,
    seed: u8,
) -> ReleaseId {
    let build = Uuid::new_v4();
    let release = ReleaseId::new();
    let receive = Uuid::new_v4();
    let source_commit = format!("{seed:040x}");
    sqlx::query("INSERT INTO build_requests (id, repository_id, source_commit, source_ref, build_definition_hash, state, created_by) VALUES ($1, $2, $3, 'refs/heads/main', $4, 'succeeded', $5)")
        .bind(build)
        .bind(repository.as_uuid())
        .bind(&source_commit)
        .bind([seed; 32].as_slice())
        .bind(actor.as_uuid())
        .execute(pool)
        .await
        .expect("seed build");
    sqlx::query("INSERT INTO releases (id, repository_id, version, source_commit, source_ref, build_request_id, build_definition_hash, configuration, configuration_hash, manifest_hash, state) VALUES ($1, $2, $3, $4, 'refs/heads/main', $5, $6, '{}', $7, $8, 'draft')")
        .bind(release.as_uuid())
        .bind(repository.as_uuid())
        .bind(format!("ui-navigation-{seed}"))
        .bind(&source_commit)
        .bind(build)
        .bind([seed; 32].as_slice())
        .bind([seed + 1; 32].as_slice())
        .bind([seed + 2; 32].as_slice())
        .execute(pool)
        .await
        .expect("seed release");
    sqlx::query("INSERT INTO git_receives (id, repository_id, actor_id, principal, status, accepted_at) VALUES ($1, $2, $3, 'ui-navigation-test', 'accepted', now())")
        .bind(receive)
        .bind(repository.as_uuid())
        .bind(actor.as_uuid())
        .execute(pool)
        .await
        .expect("seed receive");
    let source_revision = Uuid::new_v4();
    sqlx::query("INSERT INTO ui_source_manifest_revisions (id, repository_id, receive_id, source_commit, entry_kind, manifest_oid, actual_size_bytes, source_sha256, status, normalized_ui_config, normalized_ui_hash, diagnostics) VALUES ($1, $2, $3, $4, 'blob', $5, 1, $6, 'valid', $7, $8, '[]')")
        .bind(source_revision)
        .bind(repository.as_uuid())
        .bind(receive)
        .bind(&source_commit)
        .bind(format!("00000000{}", source_revision.simple()))
        .bind([3_u8; 32].as_slice())
        .bind(json!({"version": 1, "uis": []}))
        .bind([4_u8; 32].as_slice())
        .execute(pool)
        .await
        .expect("seed source revision");
    sqlx::query("INSERT INTO build_request_ui_source_manifests (build_request_id, repository_id, source_commit, source_manifest_revision_id) VALUES ($1, $2, $3, $4)")
        .bind(build)
        .bind(repository.as_uuid())
        .bind(&source_commit)
        .bind(source_revision)
        .execute(pool)
        .await
        .expect("seed source link");
    sqlx::query("INSERT INTO release_ui_source_snapshots (release_id, build_request_id, source_manifest_revision_id) VALUES ($1, $2, $3)")
        .bind(release.as_uuid())
        .bind(build)
        .bind(source_revision)
        .execute(pool)
        .await
        .expect("seed release source snapshot");
    release
}

pub async fn seed_ui_descriptor(pool: &PgPool, release: ReleaseId, key: &str, scope: &str) {
    sqlx::query("INSERT INTO release_ui_descriptors (release_id, ui_key, scope, label, icon, presentation, route_base, entrypoint, ui_kit_version, cache, content_kind) VALUES ($1, $2, $3, $2, 'app', 'full_page', $2, 'index.html', 1, 'no_store', 'static')")
        .bind(release.as_uuid())
        .bind(key)
        .bind(scope)
        .execute(pool)
        .await
        .expect("seed descriptor");
}

pub async fn install_row(
    pool: &PgPool,
    actor: UserId,
    target: UiInstallationTarget,
    key: &str,
    lifecycle: UiInstallationState,
    release: ReleaseId,
) {
    let installation = UiInstallationId::new();
    let generation = UiInstallationGenerationId::new();
    let (organization, mut project, repository, scope) = match target {
        UiInstallationTarget::Organization(id) => (Some(id.as_uuid()), None, None, "global"),
        UiInstallationTarget::Project(id) => (None, Some(id.as_uuid()), None, "project"),
        UiInstallationTarget::Repository(id) => (None, None, Some(id.as_uuid()), "repository"),
    };
    if let Some(repository_id) = repository {
        project = Some(
            sqlx::query_scalar("SELECT project_id FROM repositories WHERE id = $1")
                .bind(repository_id)
                .fetch_one(pool)
                .await
                .expect("load repository owner project"),
        );
    }
    let state = match lifecycle {
        UiInstallationState::Enabled => "enabled",
        UiInstallationState::Disabled => "disabled",
        UiInstallationState::Removed => "removed",
    };
    let mut transaction = pool.begin().await.expect("begin installation seed");
    sqlx::query("INSERT INTO ui_installations (id, organization_id, project_id, repository_id, scope, ui_key, lifecycle, current_generation_id, created_by, removed_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, CASE WHEN $7 = 'removed' THEN now() ELSE NULL END)")
        .bind(installation.as_uuid())
        .bind(organization)
        .bind(project)
        .bind(repository)
        .bind(scope)
        .bind(key)
        .bind(state)
        .bind(generation.as_uuid())
        .bind(actor.as_uuid())
        .execute(&mut *transaction)
        .await
        .expect("seed installation");
    sqlx::query("INSERT INTO ui_installation_generations (id, installation_id, generation_no, release_id, ui_key, ui_scope) VALUES ($1, $2, 1, $3, $4, $5)")
        .bind(generation.as_uuid())
        .bind(installation.as_uuid())
        .bind(release.as_uuid())
        .bind(key)
        .bind(scope)
        .execute(&mut *transaction)
        .await
        .expect("seed generation");
    transaction
        .commit()
        .await
        .expect("commit installation seed");
}
