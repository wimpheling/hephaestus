use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

pub const COMMIT_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
pub const COMMIT_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

#[derive(Clone, Copy)]
pub struct ReleaseFixture {
    pub owner: Uuid,
    pub outsider: Uuid,
    pub release: Uuid,
    pub release_without_ui: Uuid,
    pub build: Uuid,
    pub build_without_ui: Uuid,
    pub source_revision: Uuid,
    pub source_revision_without_ui: Uuid,
    pub release_agent: Uuid,
    pub foreign_agent: Uuid,
    pub artifact: Uuid,
    pub foreign_artifact: Uuid,
}
// Keep the disposable SQL fixture setup together for readable schema coverage.
#[allow(clippy::too_many_lines)]
pub async fn seed_fixture(pool: &PgPool) -> ReleaseFixture {
    let owner = Uuid::new_v4();
    let outsider = Uuid::new_v4();
    let organization = Uuid::new_v4();
    let project = Uuid::new_v4();
    let repository = Uuid::new_v4();
    let private_repository = Uuid::new_v4();
    for (id, name) in [(owner, "ui-schema-owner"), (outsider, "ui-schema-outsider")] {
        sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, $2)")
            .bind(id)
            .bind(name)
            .execute(pool)
            .await
            .expect("seed user");
    }
    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, 'ui-schema-org')")
        .bind(organization)
        .execute(pool)
        .await
        .expect("seed organization");
    sqlx::query(
        "INSERT INTO organization_members (organization_id, user_id, role)
         VALUES ($1, $2, 'member')",
    )
    .bind(organization)
    .bind(owner)
    .execute(pool)
    .await
    .expect("seed organization owner");
    sqlx::query("INSERT INTO projects (id, organization_id, name) VALUES ($1, $2, 'ui-schema')")
        .bind(project)
        .bind(organization)
        .execute(pool)
        .await
        .expect("seed project");
    sqlx::query("INSERT INTO project_maintainers (project_id, user_id) VALUES ($1, $2)")
        .bind(project)
        .bind(owner)
        .execute(pool)
        .await
        .expect("seed project maintainer");
    for (id, name) in [
        (repository, "ui-schema-repository"),
        (private_repository, "ui-schema-private"),
    ] {
        sqlx::query(
            "INSERT INTO repositories
             (id, project_id, name, default_branch, is_public)
             VALUES ($1, $2, $3, 'refs/heads/main', false)",
        )
        .bind(id)
        .bind(project)
        .bind(name)
        .execute(pool)
        .await
        .expect("seed repository");
    }

    let release = Uuid::new_v4();
    let release_without_ui = Uuid::new_v4();
    let build = Uuid::new_v4();
    let build_without_ui = Uuid::new_v4();
    let source_revision = Uuid::new_v4();
    let source_revision_without_ui = Uuid::new_v4();
    let release_agent = Uuid::new_v4();
    let foreign_agent = Uuid::new_v4();
    let artifact = Uuid::new_v4();
    let foreign_artifact = Uuid::new_v4();

    seed_release_inputs(
        pool,
        owner,
        repository,
        release,
        build,
        source_revision,
        COMMIT_A,
        release_agent,
        artifact,
        true,
    )
    .await;
    seed_release_inputs(
        pool,
        owner,
        repository,
        release_without_ui,
        build_without_ui,
        source_revision_without_ui,
        COMMIT_B,
        foreign_agent,
        foreign_artifact,
        false,
    )
    .await;
    ReleaseFixture {
        owner,
        outsider,
        release,
        release_without_ui,
        build,
        build_without_ui,
        source_revision,
        source_revision_without_ui,
        release_agent,
        foreign_agent,
        artifact,
        foreign_artifact,
    }
}

// Keep every identity key explicit so this fixture mirrors the FK graph.
#[allow(clippy::too_many_arguments)]
// Keep the release identity fixture SQL together so its FK relationships are clear.
#[allow(clippy::too_many_lines)]
pub async fn seed_release_inputs(
    pool: &PgPool,
    owner: Uuid,
    repository: Uuid,
    release: Uuid,
    build: Uuid,
    source_revision: Uuid,
    commit: &str,
    release_agent: Uuid,
    artifact: Uuid,
    include_snapshot: bool,
) {
    let receive = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO git_receives
         (id, repository_id, actor_id, principal, status, accepted_at)
         VALUES ($1, $2, $3, 'ui-schema-test', 'accepted', now())",
    )
    .bind(receive)
    .bind(repository)
    .bind(owner)
    .execute(pool)
    .await
    .expect("seed receive");
    sqlx::query(
        "INSERT INTO build_requests
         (id, repository_id, source_commit, source_ref, origin_receive_id,
          build_definition_hash, state, created_by)
         VALUES ($1, $2, $3, 'refs/heads/main', $4, $5, 'succeeded', $6)",
    )
    .bind(build)
    .bind(repository)
    .bind(commit)
    .bind(receive)
    .bind([1_u8; 32].as_slice())
    .bind(owner)
    .execute(pool)
    .await
    .expect("seed build request");
    sqlx::query(
        "INSERT INTO ui_source_manifest_revisions
         (id, repository_id, receive_id, source_commit, entry_kind, manifest_oid,
          actual_size_bytes, source_sha256, status, normalized_ui_config,
          normalized_ui_hash)
         VALUES ($1, $2, $3, $4, 'blob', $5, 32, $6, 'valid', $7, $8)",
    )
    .bind(source_revision)
    .bind(repository)
    .bind(receive)
    .bind(commit)
    .bind("cccccccccccccccccccccccccccccccccccccccc")
    .bind([2_u8; 32].as_slice())
    .bind(json!({"version": 1, "uis": []}))
    .bind([3_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("seed valid UI source capture");
    sqlx::query(
        "INSERT INTO build_request_ui_source_manifests
         (build_request_id, repository_id, source_commit, source_manifest_revision_id)
         VALUES ($1, $2, $3, $4)",
    )
    .bind(build)
    .bind(repository)
    .bind(commit)
    .bind(source_revision)
    .execute(pool)
    .await
    .expect("link build to UI source capture");
    sqlx::query(
        "INSERT INTO releases
         (id, repository_id, version, source_commit, source_ref,
          build_request_id, build_definition_hash, configuration,
          configuration_hash, manifest_hash, state)
         VALUES ($1, $2, $3, $4, 'refs/heads/main', $5, $6, $7, $8, $9, 'draft')",
    )
    .bind(release)
    .bind(repository)
    .bind(format!("v1.0.{}", &release.to_string()[..8]))
    .bind(commit)
    .bind(build)
    .bind([1_u8; 32].as_slice())
    .bind(json!({"version": 1}))
    .bind([4_u8; 32].as_slice())
    .bind([5_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("seed draft release");
    let family = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO agent_families (id, repository_id, agent_key)
         VALUES ($1, $2, $3)",
    )
    .bind(family)
    .bind(repository)
    .bind(format!("ui-agent-{}", &release.to_string()[..8]))
    .execute(pool)
    .await
    .expect("seed agent family");
    sqlx::query(
        "INSERT INTO release_agents
         (id, release_id, family_id, agent_key, display_name,
          runtime_contract, runtime_contract_hash, requires_state)
         VALUES ($1, $2, $3, $4, 'UI schema agent', '{}', $5, false)",
    )
    .bind(release_agent)
    .bind(release)
    .bind(family)
    .bind(format!("ui-agent-{}", &release.to_string()[..8]))
    .bind([6_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("seed release agent");
    sqlx::query(
        "INSERT INTO release_artifacts
         (id, release_id, path, kind, mode, content_hash, size_bytes,
          media_type, storage_key)
         VALUES ($1, $2, 'dist/index.html', 'file', 420, $3, 12,
                 'text/html', $4)",
    )
    .bind(artifact)
    .bind(release)
    .bind([7_u8; 32].as_slice())
    .bind(Uuid::new_v4())
    .execute(pool)
    .await
    .expect("seed release artifact");
    if include_snapshot {
        sqlx::query(
            "INSERT INTO release_ui_source_snapshots
             (release_id, build_request_id, source_manifest_revision_id)
             VALUES ($1, $2, $3)",
        )
        .bind(release)
        .bind(build)
        .bind(source_revision)
        .execute(pool)
        .await
        .expect("seed release UI source snapshot");
    }
}
