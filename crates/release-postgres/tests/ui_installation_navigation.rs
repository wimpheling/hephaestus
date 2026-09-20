//! Real-PostgreSQL navigation projection matrix candidate.

use forge_domain::{OrganizationId, ProjectId, RepositoryId};
use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
use release_domain::{
    ReleaseId, UiInstallationGenerationId, UiInstallationId, UiInstallationState,
    UiInstallationTarget,
};
use release_postgres::PgUiInstallationNavigator;
use release_service::{
    ListUiInstallations, UiInstallationNavigator, UiInstallationPage, UiInstallationTargetFilter,
};
use serde_json::json;
use serial_test::serial;
use sqlx::{PgPool, postgres::PgPoolOptions};
use uuid::Uuid;

#[tokio::test]
#[serial]
// Keep the authority, tenancy, lifecycle, and cursor assertions in one
// disposable real-PostgreSQL fixture so every result shares the same actor.
#[allow(clippy::too_many_lines)]
async fn list_ui_installations_enforces_explicit_org_target_and_visibility() {
    let Ok(database_url) = std::env::var("HEPHAESTUS_POSTGRES_TEST_URL") else {
        eprintln!("skipping UI navigation projection: test URL is unset");
        return;
    };
    let bootstrap = PgPoolOptions::new()
        .max_connections(8)
        .connect(&database_url)
        .await
        .expect("connect PostgreSQL bootstrap pool");
    sqlx::migrate!("../../migrations")
        .run(&bootstrap)
        .await
        .expect("apply migrations");

    let actor = UserId::new();
    let organization = OrganizationId::new();
    let project = ProjectId::new();
    let repository = RepositoryId::new();
    let source_project = ProjectId::new();
    let source_repository = RepositoryId::new();
    let second_organization = OrganizationId::new();
    let second_project = ProjectId::new();
    let second_repository = RepositoryId::new();
    seed_owner_and_target(&bootstrap, actor, organization, project, repository).await;
    seed_source_target(
        &bootstrap,
        actor,
        organization,
        source_project,
        source_repository,
    )
    .await;
    seed_owner_and_target(
        &bootstrap,
        actor,
        second_organization,
        second_project,
        second_repository,
    )
    .await;
    let release = seed_published_release(&bootstrap, actor, source_repository, 1).await;
    let second_release = seed_published_release(&bootstrap, actor, second_repository, 2).await;
    seed_ui_descriptor(&bootstrap, release, "project-ui", "project").await;
    seed_ui_descriptor(&bootstrap, release, "project-next", "project").await;
    seed_ui_descriptor(&bootstrap, release, "project-live", "project").await;
    seed_ui_descriptor(&bootstrap, release, "repository-ui", "repository").await;
    seed_ui_descriptor(&bootstrap, release, "global-ui", "global").await;
    seed_ui_descriptor(&bootstrap, second_release, "global-ui", "global").await;
    install_row(
        &bootstrap,
        actor,
        UiInstallationTarget::project(project),
        "project-ui",
        UiInstallationState::Removed,
        release,
    )
    .await;
    install_row(
        &bootstrap,
        actor,
        UiInstallationTarget::project(project),
        "project-live",
        UiInstallationState::Enabled,
        release,
    )
    .await;
    install_row(
        &bootstrap,
        actor,
        UiInstallationTarget::project(project),
        "project-next",
        UiInstallationState::Enabled,
        release,
    )
    .await;
    install_row(
        &bootstrap,
        actor,
        UiInstallationTarget::repository(repository),
        "repository-ui",
        UiInstallationState::Enabled,
        release,
    )
    .await;
    install_row(
        &bootstrap,
        actor,
        UiInstallationTarget::organization(organization),
        "global-ui",
        UiInstallationState::Disabled,
        release,
    )
    .await;
    install_row(
        &bootstrap,
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
            .execute(&bootstrap)
            .await
            .expect("publish source release after immutable fixture inserts");
    }

    let app_pool = PgPoolOptions::new()
        .max_connections(4)
        .after_connect(|connection, _metadata| {
            Box::pin(async move {
                sqlx::query("SET ROLE hephaestus_app")
                    .execute(&mut *connection)
                    .await
                    .map(|_| ())
            })
        })
        .connect(&database_url)
        .await
        .expect("connect application-role pool");
    let navigator = PgUiInstallationNavigator::new(app_pool.clone());
    let identity = identity(actor);
    let invalid_zero_page = navigator
        .list_ui_installations(
            &identity,
            ListUiInstallations {
                organization_id: organization,
                target: UiInstallationTargetFilter::Project(project),
                page: UiInstallationPage {
                    size: 0,
                    after: None,
                },
            },
        )
        .await;
    assert!(matches!(
        invalid_zero_page,
        Err(release_service::UiInstallationNavigationError::InvalidPage)
    ));
    let invalid_large_page = navigator
        .list_ui_installations(
            &identity,
            ListUiInstallations {
                organization_id: organization,
                target: UiInstallationTargetFilter::Project(project),
                page: UiInstallationPage {
                    size: 101,
                    after: None,
                },
            },
        )
        .await;
    assert!(matches!(
        invalid_large_page,
        Err(release_service::UiInstallationNavigationError::InvalidPage)
    ));
    let project_page = navigator
        .list_ui_installations(
            &identity,
            ListUiInstallations {
                organization_id: organization,
                target: UiInstallationTargetFilter::Project(project),
                page: UiInstallationPage {
                    size: 1,
                    after: None,
                },
            },
        )
        .await
        .expect("first project navigation page");
    assert_eq!(project_page.installations.len(), 1);
    assert_eq!(
        project_page.installations[0].lifecycle,
        UiInstallationState::Enabled
    );
    let project_second_page = navigator
        .list_ui_installations(
            &identity,
            ListUiInstallations {
                organization_id: organization,
                target: UiInstallationTargetFilter::Project(project),
                page: UiInstallationPage {
                    size: 1,
                    after: project_page.next,
                },
            },
        )
        .await
        .expect("second project navigation page");
    assert_eq!(project_second_page.installations.len(), 1);
    assert!(project_second_page.next.is_none());
    assert!(
        [
            project_page.installations[0].ui_key.as_str(),
            project_second_page.installations[0].ui_key.as_str(),
        ]
        .into_iter()
        .all(|key| key != "project-ui")
    );

    let repository_page = navigator
        .list_ui_installations(
            &identity,
            ListUiInstallations {
                organization_id: organization,
                target: UiInstallationTargetFilter::Repository(repository),
                page: UiInstallationPage::default(),
            },
        )
        .await
        .expect("repository navigation projection");
    assert_eq!(repository_page.installations.len(), 1);
    assert_eq!(
        repository_page.installations[0].lifecycle,
        UiInstallationState::Enabled
    );
    assert!(repository_page.installations[0].launchable);

    let global_page = navigator
        .list_ui_installations(
            &identity,
            ListUiInstallations {
                organization_id: organization,
                target: UiInstallationTargetFilter::Global,
                page: UiInstallationPage::default(),
            },
        )
        .await
        .expect("global navigation projection");
    assert_eq!(global_page.installations.len(), 1);
    assert_eq!(
        global_page.installations[0].lifecycle,
        UiInstallationState::Disabled
    );
    assert!(!global_page.installations[0].launchable);

    let second_global_page = navigator
        .list_ui_installations(
            &identity,
            ListUiInstallations {
                organization_id: second_organization,
                target: UiInstallationTargetFilter::Global,
                page: UiInstallationPage::default(),
            },
        )
        .await
        .expect("second organization navigation projection");
    assert_eq!(second_global_page.installations.len(), 1);
    assert_eq!(
        second_global_page.installations[0].lifecycle,
        UiInstallationState::Enabled
    );

    let foreign_target_page = navigator
        .list_ui_installations(
            &identity,
            ListUiInstallations {
                organization_id: second_organization,
                target: UiInstallationTargetFilter::Repository(repository),
                page: UiInstallationPage::default(),
            },
        )
        .await;
    assert!(matches!(
        foreign_target_page,
        Err(release_service::UiInstallationNavigationError::InvalidPage)
    ));

    let foreign_cursor = navigator
        .list_ui_installations(
            &identity,
            ListUiInstallations {
                organization_id: organization,
                target: UiInstallationTargetFilter::Repository(repository),
                page: UiInstallationPage {
                    size: 1,
                    after: Some(global_page.installations[0].installation_id),
                },
            },
        )
        .await;
    assert!(matches!(
        foreign_cursor,
        Err(release_service::UiInstallationNavigationError::InvalidPage)
    ));

    sqlx::query("UPDATE releases SET state = 'revoked', revoked_at = now() WHERE id = $1")
        .bind(release.as_uuid())
        .execute(&bootstrap)
        .await
        .expect("revoke source release use permission");
    let source_use_revoked = navigator
        .list_ui_installations(
            &identity,
            ListUiInstallations {
                organization_id: organization,
                target: UiInstallationTargetFilter::Repository(repository),
                page: UiInstallationPage::default(),
            },
        )
        .await
        .expect("source use revocation remains a safe projection");
    assert_eq!(source_use_revoked.installations.len(), 1);
    assert!(!source_use_revoked.installations[0].launchable);

    sqlx::query("DELETE FROM project_maintainers WHERE project_id = $1 AND user_id = $2")
        .bind(source_project.as_uuid())
        .bind(actor.as_uuid())
        .execute(&bootstrap)
        .await
        .expect("revoke source project read permission");
    sqlx::query("DELETE FROM organization_members WHERE organization_id = $1 AND user_id = $2")
        .bind(organization.as_uuid())
        .bind(actor.as_uuid())
        .execute(&bootstrap)
        .await
        .expect("revoke current organization read permission");
    let revoked_page = navigator
        .list_ui_installations(
            &identity,
            ListUiInstallations {
                organization_id: organization,
                target: UiInstallationTargetFilter::Repository(repository),
                page: UiInstallationPage::default(),
            },
        )
        .await
        .expect("revoked navigation remains a successful empty read");
    assert!(revoked_page.installations.is_empty());
    let other_org_remains_visible = navigator
        .list_ui_installations(
            &identity,
            ListUiInstallations {
                organization_id: second_organization,
                target: UiInstallationTargetFilter::Global,
                page: UiInstallationPage::default(),
            },
        )
        .await
        .expect("other organization remains independently visible");
    assert_eq!(other_org_remains_visible.installations.len(), 1);
    println!(
        "REAL_UI_NAVIGATION=1 app_role=1 explicit_targets=1 project_repo_global=1 dual_org=1 pagination=1 invalid_pages=1 removed_excluded=1 source_use_launchable_false=1 source_read_hidden=1 owner_revocation_other_org_visible=1"
    );
}

async fn seed_owner_and_target(
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

async fn seed_source_target(
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

async fn seed_published_release(
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

async fn seed_ui_descriptor(pool: &PgPool, release: ReleaseId, key: &str, scope: &str) {
    sqlx::query("INSERT INTO release_ui_descriptors (release_id, ui_key, scope, label, icon, presentation, route_base, entrypoint, ui_kit_version, cache, content_kind) VALUES ($1, $2, $3, $2, 'app', 'full_page', $2, 'index.html', 1, 'no_store', 'static')")
        .bind(release.as_uuid())
        .bind(key)
        .bind(scope)
        .execute(pool)
        .await
        .expect("seed descriptor");
}

async fn install_row(
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

fn identity(user_id: UserId) -> AuthenticatedIdentity {
    AuthenticatedIdentity::new(
        user_id,
        "https://ui-navigation-test.example",
        format!("subject-{user_id}"),
        json!({"email_verified": true}),
        RequestId::new(),
    )
}
