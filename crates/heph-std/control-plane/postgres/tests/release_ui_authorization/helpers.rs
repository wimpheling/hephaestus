//! Release UI authorization fixture helpers.

use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

// These names mirror the database columns to keep fixture bindings auditable.
#[allow(clippy::struct_field_names)]
#[derive(Clone)]
pub(super) struct ReleaseFixture {
    pub(crate) build_id: Uuid,
    pub(crate) release_id: Uuid,
    pub(crate) receive_id: Uuid,
    pub(crate) revision_id: Uuid,
}

pub(super) async fn seed_identity_and_repository(
    pool: &PgPool,
    owner: UserId,
    outsider: UserId,
    organization: Uuid,
    project: Uuid,
    repository: Uuid,
) {
    for (user, name) in [(owner, "ui-owner"), (outsider, "ui-outsider")] {
        sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, $2)")
            .bind(user.as_uuid())
            .bind(name)
            .execute(pool)
            .await
            .expect("seed UI user");
    }
    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, 'ui-org')")
        .bind(organization)
        .execute(pool)
        .await
        .expect("seed UI organization");
    sqlx::query(
        "INSERT INTO organization_members (organization_id, user_id, role)
         VALUES ($1, $2, 'owner')",
    )
    .bind(organization)
    .bind(owner.as_uuid())
    .execute(pool)
    .await
    .expect("seed UI owner membership");
    sqlx::query("INSERT INTO projects (id, organization_id, name) VALUES ($1, $2, 'ui-project')")
        .bind(project)
        .bind(organization)
        .execute(pool)
        .await
        .expect("seed UI project");
    sqlx::query("INSERT INTO repositories (id, project_id, name) VALUES ($1, $2, 'ui-repository')")
        .bind(repository)
        .bind(project)
        .execute(pool)
        .await
        .expect("seed UI repository");
}

pub(super) async fn seed_release(
    pool: &PgPool,
    owner: UserId,
    repository: Uuid,
    seed: u8,
) -> ReleaseFixture {
    let build_id = Uuid::new_v4();
    let release_id = Uuid::new_v4();
    let receive_id = Uuid::new_v4();
    let revision_id = Uuid::new_v4();
    let source_commit = format!("{seed:040x}");
    sqlx::query(
        "INSERT INTO build_requests
         (id, repository_id, source_commit, source_ref, build_definition_hash, state, created_by)
         VALUES ($1, $2, $3, 'refs/heads/main', $4, 'succeeded', $5)",
    )
    .bind(build_id)
    .bind(repository)
    .bind(&source_commit)
    .bind([seed; 32].as_slice())
    .bind(owner.as_uuid())
    .execute(pool)
    .await
    .expect("seed UI build");
    sqlx::query(
        "INSERT INTO releases
         (id, repository_id, version, source_commit, source_ref, build_request_id,
          build_definition_hash, configuration, configuration_hash, manifest_hash, state)
         VALUES ($1, $2, $3, $4, 'refs/heads/main', $5, $6, '{}', $7, $8, 'draft')",
    )
    .bind(release_id)
    .bind(repository)
    .bind(format!("v1.0.{seed}"))
    .bind(&source_commit)
    .bind(build_id)
    .bind([seed; 32].as_slice())
    .bind([seed + 10; 32].as_slice())
    .bind([seed + 20; 32].as_slice())
    .execute(pool)
    .await
    .expect("seed UI release");
    sqlx::query(
        "INSERT INTO git_receives
         (id, repository_id, actor_id, principal, status, accepted_at)
         VALUES ($1, $2, $3, 'release-ui-test', 'accepted', now())",
    )
    .bind(receive_id)
    .bind(repository)
    .bind(owner.as_uuid())
    .execute(pool)
    .await
    .expect("seed UI receive");
    ReleaseFixture {
        build_id,
        release_id,
        receive_id,
        revision_id,
    }
}

pub(super) async fn seed_ui_source(pool: &PgPool, fixture: &ReleaseFixture) {
    sqlx::query(
        "INSERT INTO ui_source_manifest_revisions
         (id, repository_id, receive_id, source_commit, entry_kind, manifest_oid,
          actual_size_bytes, source_sha256, status, normalized_ui_config,
          normalized_ui_hash, diagnostics)
         SELECT $1, repository_id, $2, source_commit, 'blob', $3,
                1, $4, 'valid', $5, $6, '[]'
         FROM build_requests WHERE id = $7",
    )
    .bind(fixture.revision_id)
    .bind(fixture.receive_id)
    .bind(format!("{:040x}", fixture.revision_id.as_u128()))
    .bind([3_u8; 32].as_slice())
    .bind(json!({"version": 1, "uis": []}))
    .bind([4_u8; 32].as_slice())
    .bind(fixture.build_id)
    .execute(pool)
    .await
    .expect("seed UI source revision");
    sqlx::query(
        "INSERT INTO build_request_ui_source_manifests
         (build_request_id, repository_id, source_commit, source_manifest_revision_id)
         SELECT $1, repository_id, source_commit, $2
         FROM build_requests WHERE id = $1",
    )
    .bind(fixture.build_id)
    .bind(fixture.revision_id)
    .execute(pool)
    .await
    .expect("seed UI source link");
    sqlx::query(
        "INSERT INTO release_ui_source_snapshots
         (release_id, build_request_id, source_manifest_revision_id)
         VALUES ($1, $2, $3)",
    )
    .bind(fixture.release_id)
    .bind(fixture.build_id)
    .bind(fixture.revision_id)
    .execute(pool)
    .await
    .expect("seed UI release source snapshot");
}

pub(super) async fn seed_descriptor(
    pool: &PgPool,
    release_id: Uuid,
    key: &str,
    content_kind: &str,
    route_base: &str,
    entrypoint: &str,
) {
    sqlx::query(
        "INSERT INTO release_ui_descriptors
         (release_id, ui_key, scope, label, icon, presentation, route_base,
          entrypoint, ui_kit_version, cache, content_kind)
         VALUES ($1, $2, 'repository', $3, 'app', 'full_page', $4, $5, 1,
                 'no_store', $6)",
    )
    .bind(release_id)
    .bind(key)
    .bind(key)
    .bind(route_base)
    .bind(entrypoint)
    .bind(content_kind)
    .execute(pool)
    .await
    .expect("seed UI descriptor");
}

pub(super) async fn seed_artifact(
    pool: &PgPool,
    release_id: Uuid,
    seed: u8,
    media_type: &str,
) -> Uuid {
    let artifact_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO release_artifacts
         (id, release_id, path, kind, mode, content_hash, size_bytes, media_type, storage_key)
         VALUES ($1, $2, $3, 'file', 420, $4, 1, $5, $6)",
    )
    .bind(artifact_id)
    .bind(release_id)
    .bind(format!("ui-{seed}.html"))
    .bind([seed; 32].as_slice())
    .bind(media_type)
    .bind(Uuid::new_v4())
    .execute(pool)
    .await
    .expect("seed UI artifact");
    artifact_id
}

pub(super) async fn seed_agent(
    pool: &PgPool,
    repository: Uuid,
    release_id: Uuid,
    seed: u8,
) -> Uuid {
    let family_id = Uuid::new_v4();
    let agent_id = Uuid::new_v4();
    sqlx::query("INSERT INTO agent_families (id, repository_id, agent_key) VALUES ($1, $2, $3)")
        .bind(family_id)
        .bind(repository)
        .bind(format!("ui-agent-{seed}"))
        .execute(pool)
        .await
        .expect("seed UI agent family");
    sqlx::query(
        "INSERT INTO release_agents
         (id, release_id, family_id, agent_key, display_name, runtime_contract,
          runtime_contract_hash, parameter_schema, secret_slot_schema, requires_state)
         VALUES ($1, $2, $3, $4, 'UI Agent', $5, $6, '[]', '[]', false)",
    )
    .bind(agent_id)
    .bind(release_id)
    .bind(family_id)
    .bind(format!("ui-agent-{seed}"))
    .bind(json!({
        "policy_ceiling": {"vcpus": 1, "memory_mib": 128, "network": "disabled"}
    }))
    .bind([seed; 32].as_slice())
    .execute(pool)
    .await
    .expect("seed UI release agent");
    agent_id
}

pub(super) async fn publish(pool: &PgPool, release_id: Uuid, owner: UserId) {
    sqlx::query(
        "UPDATE releases
         SET state = 'published', publication_actor_id = $2, published_at = now()
         WHERE id = $1",
    )
    .bind(release_id)
    .bind(owner.as_uuid())
    .execute(pool)
    .await
    .expect("publish UI test release");
}

pub(super) fn identity(user_id: UserId) -> AuthenticatedIdentity {
    AuthenticatedIdentity::new(
        user_id,
        "https://release-ui-test.example",
        format!("subject-{user_id}"),
        json!({"email_verified": true}),
        RequestId::new(),
    )
}
