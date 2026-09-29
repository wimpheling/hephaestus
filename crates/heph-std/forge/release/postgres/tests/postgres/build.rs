use super::*;

pub(crate) fn install_command(
    key: &str,
    target: UiInstallationTarget,
    release_id: ReleaseId,
    ui_key: &str,
) -> InstallStaticUi {
    InstallStaticUi {
        caller_key: UiInstallationCallerKey::parse(key).expect("caller key"),
        target,
        release_id,
        ui_key: release_domain::ui::UiKey::parse(ui_key).expect("UI key"),
    }
}

pub(crate) async fn publish_static_release(
    admin_pool: &PgPool,
    worker_pool: &PgPool,
    fixture: &Fixture,
    scope: &str,
    label: &str,
) -> ReleaseId {
    let release_id = ReleaseId::new();
    let build = clone_build_for_ui_capture(admin_pool, fixture).await;
    let manifest = static_ui_manifest_for_scope(scope);
    attach_ui_capture(admin_pool, build, manifest, None, None).await;
    ReleaseService::new(worker_pool.clone(), Arc::new(PostgresMelangeAuthorizer))
        .complete_build(CompleteBuild {
            command_key: key(label, release_id.as_uuid()),
            build_request_id: build,
            release_id,
            version: ReleaseVersion::parse(format!("{label}-v1")).expect("release version"),
            release_agent_id: ReleaseAgentId::new(),
            artifacts: vec![release_test_artifact(
                "dist/index.html",
                ArtifactKind::File,
                "text/html",
                10,
            )],
        })
        .await
        .expect("complete static installation release");
    ReleaseService::new(admin_pool.clone(), Arc::new(PostgresMelangeAuthorizer))
        .publish(
            &identity(fixture.actor),
            key("publish-installation-static", release_id.as_uuid()),
            release_id,
        )
        .await
        .expect("publish static installation release");
    release_id
}

pub(crate) async fn publish_static_api_release(
    admin_pool: &PgPool,
    worker_pool: &PgPool,
    fixture: &Fixture,
) -> ReleaseId {
    let release_id = ReleaseId::new();
    let build = clone_build_for_ui_capture(admin_pool, fixture).await;
    let manifest = static_ui_api_manifest();
    attach_ui_capture(
        admin_pool,
        build,
        manifest,
        Some(managed_gateway_manifest("reviewer").as_bytes()),
        None,
    )
    .await;
    ReleaseService::new(worker_pool.clone(), Arc::new(PostgresMelangeAuthorizer))
        .complete_build(CompleteBuild {
            command_key: key("complete-static-api", release_id.as_uuid()),
            build_request_id: build,
            release_id,
            version: ReleaseVersion::parse("static-api-v1").expect("release version"),
            release_agent_id: ReleaseAgentId::new(),
            artifacts: vec![release_test_artifact(
                "dist/index.html",
                ArtifactKind::File,
                "text/html",
                10,
            )],
        })
        .await
        .expect("complete static API release");
    ReleaseService::new(admin_pool.clone(), Arc::new(PostgresMelangeAuthorizer))
        .publish(
            &identity(fixture.actor),
            key("publish-installation-static-api", release_id.as_uuid()),
            release_id,
        )
        .await
        .expect("publish static API release");
    release_id
}

pub(crate) async fn publish_managed_release(
    admin_pool: &PgPool,
    worker_pool: &PgPool,
    fixture: &Fixture,
) -> ReleaseId {
    let release_id = ReleaseId::new();
    let build = clone_build_for_ui_capture(admin_pool, fixture).await;
    attach_ui_capture(
        admin_pool,
        build,
        managed_api_manifest(),
        Some(managed_gateway_manifest("reviewer").as_bytes()),
        None,
    )
    .await;
    ReleaseService::new(worker_pool.clone(), Arc::new(PostgresMelangeAuthorizer))
        .complete_build(CompleteBuild {
            command_key: key("complete-managed-installation", release_id.as_uuid()),
            build_request_id: build,
            release_id,
            version: ReleaseVersion::parse("managed-install-v1").expect("release version"),
            release_agent_id: ReleaseAgentId::new(),
            artifacts: vec![release_test_artifact(
                "bin/reviewer",
                ArtifactKind::Executable,
                "application/octet-stream",
                20,
            )],
        })
        .await
        .expect("complete managed installation release");
    ReleaseService::new(admin_pool.clone(), Arc::new(PostgresMelangeAuthorizer))
        .publish(
            &identity(fixture.actor),
            key("publish-installation-managed", release_id.as_uuid()),
            release_id,
        )
        .await
        .expect("publish managed installation release");
    release_id
}

pub(crate) async fn publish_managed_release_for_scope(
    admin_pool: &PgPool,
    worker_pool: &PgPool,
    fixture: &Fixture,
    scope: &str,
) -> ReleaseId {
    assert!(matches!(scope, "global" | "project" | "repository"));
    let release_id = ReleaseId::new();
    let build = clone_build_for_ui_capture(admin_pool, fixture).await;
    let manifest = MANAGED_API_UI.replace("scope = \"project\"", &format!("scope = \"{scope}\""));
    attach_ui_capture(
        admin_pool,
        build,
        manifest,
        Some(managed_gateway_manifest("reviewer").as_bytes()),
        None,
    )
    .await;
    ReleaseService::new(worker_pool.clone(), Arc::new(PostgresMelangeAuthorizer))
        .complete_build(CompleteBuild {
            command_key: key("complete-managed-installation-scope", release_id.as_uuid()),
            build_request_id: build,
            release_id,
            version: ReleaseVersion::parse("managed-install-scope-v1").expect("release version"),
            release_agent_id: ReleaseAgentId::new(),
            artifacts: vec![release_test_artifact(
                "bin/reviewer",
                ArtifactKind::Executable,
                "application/octet-stream",
                20,
            )],
        })
        .await
        .expect("complete scoped managed installation release");
    ReleaseService::new(admin_pool.clone(), Arc::new(PostgresMelangeAuthorizer))
        .publish(
            &identity(fixture.actor),
            key("publish-managed-installation-scope", release_id.as_uuid()),
            release_id,
        )
        .await
        .expect("publish scoped managed installation release");
    release_id
}

// Each published release needs its own immutable source capture. Cloning the
// reusable build keeps release fixtures independent while preserving the same
// repository, actor, and validated configuration.
pub(crate) async fn clone_build_for_ui_capture(
    admin_pool: &PgPool,
    fixture: &Fixture,
) -> BuildRequestId {
    let build = BuildRequestId::new();
    let commit = format!("{}00000000", Uuid::new_v4().simple());
    let (repository_id, receive_id, source_ref, build_definition_hash, created_by): (
        Uuid,
        Uuid,
        String,
        Vec<u8>,
        Uuid,
    ) = sqlx::query_as(
        "SELECT repository_id, origin_receive_id, source_ref,
                    build_definition_hash, created_by
             FROM build_requests WHERE id = $1",
    )
    .bind(fixture.build.as_uuid())
    .fetch_one(admin_pool)
    .await
    .expect("load reusable build for UI capture");
    sqlx::query(
        "INSERT INTO agent_config_revisions
         (id, repository_id, receive_id, commit_sha, config_hash,
          normalized_config_hash, schema_version, status, config, diagnostics)
         SELECT $1, repository_id, receive_id, $2, config_hash,
                normalized_config_hash, schema_version, status, config, diagnostics
         FROM agent_config_revisions
         WHERE repository_id = $3
           AND commit_sha = (SELECT source_commit FROM build_requests WHERE id = $4)",
    )
    .bind(Uuid::new_v4())
    .bind(&commit)
    .bind(repository_id)
    .bind(fixture.build.as_uuid())
    .execute(admin_pool)
    .await
    .expect("clone reusable configuration revision");
    sqlx::query(
        "INSERT INTO build_requests
         (id, repository_id, source_commit, source_ref, origin_receive_id,
          build_definition_hash, state, created_by)
         VALUES ($1, $2, $3, $4, $5, $6, 'importing', $7)",
    )
    .bind(build.as_uuid())
    .bind(repository_id)
    .bind(&commit)
    .bind(source_ref)
    .bind(receive_id)
    .bind(build_definition_hash)
    .bind(created_by)
    .execute(admin_pool)
    .await
    .expect("clone reusable build request");
    sqlx::query(
        "INSERT INTO build_request_images
         (build_request_id, execution_context, image_id, image_key, image_reference)
         SELECT $1, execution_context, image_id, image_key, image_reference
         FROM build_request_images WHERE build_request_id = $2",
    )
    .bind(build.as_uuid())
    .bind(fixture.build.as_uuid())
    .execute(admin_pool)
    .await
    .expect("clone reusable build images");
    build
}

pub(crate) async fn seed_foreign_project(pool: &PgPool, actor: UserId) -> ProjectId {
    let organization = OrganizationId::new();
    let project = ProjectId::new();
    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, $2)")
        .bind(organization.as_uuid())
        .bind(format!("foreign-ui-org-{organization}"))
        .execute(pool)
        .await
        .expect("seed foreign organization");
    sqlx::query(
        "INSERT INTO organization_members (organization_id, user_id, role)
         VALUES ($1, $2, 'member')",
    )
    .bind(organization.as_uuid())
    .bind(actor.as_uuid())
    .execute(pool)
    .await
    .expect("seed foreign organization membership");
    sqlx::query("INSERT INTO projects (id, organization_id, name) VALUES ($1, $2, $3)")
        .bind(project.as_uuid())
        .bind(organization.as_uuid())
        .bind(format!("foreign-ui-project-{project}"))
        .execute(pool)
        .await
        .expect("seed foreign project");
    sqlx::query("INSERT INTO project_maintainers (project_id, user_id) VALUES ($1, $2)")
        .bind(project.as_uuid())
        .bind(actor.as_uuid())
        .execute(pool)
        .await
        .expect("seed foreign project maintainer");
    project
}
