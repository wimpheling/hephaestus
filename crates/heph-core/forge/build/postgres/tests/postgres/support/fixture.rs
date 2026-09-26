//! Relational build fixture and durable request copying.

use release_domain::BuildRequestId;
use uuid::Uuid;

use super::config::CONFIG;

pub async fn copy_build_request(
    pool: &sqlx::PgPool,
    source: BuildRequestId,
    destination: BuildRequestId,
    definition_hash: [u8; 32],
) {
    sqlx::query(
        "INSERT INTO build_requests
         (id, repository_id, source_commit, source_ref, origin_receive_id,
          build_definition_hash, state, created_by)
         SELECT $1, repository_id, source_commit, source_ref, origin_receive_id,
                $2, 'queued', created_by
         FROM build_requests WHERE id = $3",
    )
    .bind(destination.as_uuid())
    .bind(definition_hash.as_slice())
    .bind(source.as_uuid())
    .execute(pool)
    .await
    .expect("copy build request");
    sqlx::query(
        "INSERT INTO build_request_images
         (build_request_id, execution_context, image_id, image_key, image_reference)
         SELECT $1, execution_context, image_id, image_key, image_reference
           FROM build_request_images WHERE build_request_id = $2",
    )
    .bind(destination.as_uuid())
    .bind(source.as_uuid())
    .execute(pool)
    .await
    .expect("copy image snapshots");
}

// One explicit fixture keeps the complete immutable build and image snapshot
// provenance visible to this integration test.
#[allow(clippy::too_many_lines)]
pub async fn seed(pool: &sqlx::PgPool, repository_id: Uuid, commit: &str) -> BuildRequestId {
    let user = Uuid::new_v4();
    let organization = Uuid::new_v4();
    let project = Uuid::new_v4();
    let receive = Uuid::new_v4();
    let build = BuildRequestId::new();
    sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, 'Build Owner')")
        .bind(user)
        .execute(pool)
        .await
        .expect("user");
    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, $2)")
        .bind(organization)
        .bind(format!("build-{organization}"))
        .execute(pool)
        .await
        .expect("organization");
    sqlx::query(
        "INSERT INTO organization_members (organization_id, user_id, role)
         VALUES ($1, $2, 'owner')",
    )
    .bind(organization)
    .bind(user)
    .execute(pool)
    .await
    .expect("owner");
    sqlx::query("INSERT INTO projects (id, organization_id, name) VALUES ($1, $2, $3)")
        .bind(project)
        .bind(organization)
        .bind(format!("project-{project}"))
        .execute(pool)
        .await
        .expect("project");
    sqlx::query("INSERT INTO project_maintainers (project_id, user_id) VALUES ($1, $2)")
        .bind(project)
        .bind(user)
        .execute(pool)
        .await
        .expect("project maintainer");
    sqlx::query(
        "INSERT INTO repositories
         (id, project_id, name, default_branch, is_public)
         VALUES ($1, $2, $3, 'refs/heads/main', false)",
    )
    .bind(repository_id)
    .bind(project)
    .bind(format!("repository-{repository_id}"))
    .execute(pool)
    .await
    .expect("repository");
    for (key, reference) in [
        (
            "build",
            "build@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        ),
        (
            "run",
            "run@sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        ),
    ] {
        sqlx::query(
            "INSERT INTO oci_images
             (id, key, display_name, image_reference, toolchains, architectures,
              availability_state, provenance, platform_policy_version)
             VALUES ($1, $2, $2, $3, '[]'::jsonb, ARRAY['x86_64'],
                     'available', '{}'::jsonb, 'test/v1')",
        )
        .bind(Uuid::new_v4())
        .bind(key)
        .bind(reference)
        .execute(pool)
        .await
        .expect("OCI image");
    }
    sqlx::query(
        "INSERT INTO git_receives
         (id, repository_id, actor_id, principal, status, accepted_at)
         VALUES ($1, $2, $3, 'build-test', 'accepted', now())",
    )
    .bind(receive)
    .bind(repository_id)
    .bind(user)
    .execute(pool)
    .await
    .expect("receive");
    let parsed = agent_config::parse(CONFIG.as_bytes());
    let config = parsed.config.expect("valid build config");
    sqlx::query(
        "INSERT INTO agent_config_revisions
         (id, repository_id, receive_id, commit_sha, config_hash,
          normalized_config_hash, schema_version, status, config, diagnostics)
         VALUES ($1, $2, $3, $4, $5, $6, 2, 'valid', $7, '[]')",
    )
    .bind(Uuid::new_v4())
    .bind(repository_id)
    .bind(receive)
    .bind(commit)
    .bind(parsed.hash.as_str())
    .bind(parsed.normalized_hash.expect("normalized hash").as_str())
    .bind(serde_json::to_value(config).expect("serialize config"))
    .execute(pool)
    .await
    .expect("config revision");
    sqlx::query(
        "INSERT INTO build_requests
         (id, repository_id, source_commit, source_ref, origin_receive_id,
          build_definition_hash, state, created_by)
         VALUES ($1, $2, $3, 'refs/heads/main', $4, $5, 'queued', $6)",
    )
    .bind(build.as_uuid())
    .bind(repository_id)
    .bind(commit)
    .bind(receive)
    .bind([9_u8; 32].as_slice())
    .bind(user)
    .execute(pool)
    .await
    .expect("build request");
    sqlx::query(
        "INSERT INTO build_request_images
         (build_request_id, execution_context, image_id, image_key, image_reference)
         SELECT $1, context.execution_context, image.id, image.key, image.image_reference
           FROM (VALUES ('build'::text, 'build'::text), ('guest', 'run'))
                    AS context(execution_context, image_key)
           JOIN oci_images AS image ON image.key = context.image_key",
    )
    .bind(build.as_uuid())
    .execute(pool)
    .await
    .expect("build image snapshots");
    build
}
