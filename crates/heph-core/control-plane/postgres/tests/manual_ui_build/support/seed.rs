use identity_domain::UserId;
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

pub async fn seed_identity(
    pool: &PgPool,
    owner: UserId,
    organization: Uuid,
    project: Uuid,
    repository: Uuid,
) {
    sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, 'manual-ui-owner')")
        .bind(owner.as_uuid())
        .execute(pool)
        .await
        .expect("user");
    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, 'manual-ui-org')")
        .bind(organization)
        .execute(pool)
        .await
        .expect("organization");
    sqlx::query(
        "INSERT INTO organization_members (organization_id, user_id, role)
         VALUES ($1, $2, 'owner')",
    )
    .bind(organization)
    .bind(owner.as_uuid())
    .execute(pool)
    .await
    .expect("organization membership");
    sqlx::query(
        "INSERT INTO projects (id, organization_id, name) VALUES ($1, $2, 'manual-ui-project')",
    )
    .bind(project)
    .bind(organization)
    .execute(pool)
    .await
    .expect("project");
    sqlx::query(
        "INSERT INTO repositories (id, project_id, name) VALUES ($1, $2, 'manual-ui-repository')",
    )
    .bind(repository)
    .bind(project)
    .execute(pool)
    .await
    .expect("repository");
    sqlx::query(
        "INSERT INTO repository_managers (repository_id, user_id)
         VALUES ($1, $2)",
    )
    .bind(repository)
    .bind(owner.as_uuid())
    .execute(pool)
    .await
    .expect("repository manager");
}

pub async fn seed_images(pool: &PgPool) {
    for (key, byte) in [("manual-builder", b'a'), ("manual-runtime", b'b')] {
        let reference = format!(
            "manual-{key}@sha256:{}",
            char::from(byte).to_string().repeat(64)
        );
        sqlx::query(
            "INSERT INTO oci_images
             (id, key, display_name, image_reference, toolchains, architectures,
              availability_state, provenance, platform_policy_version, role)
             VALUES ($1, $2, $3, $4, '[]', ARRAY['x86_64'], 'available', '{}', 'test/v1', 'execution')",
        )
        .bind(Uuid::new_v4())
        .bind(key)
        .bind(key)
        .bind(reference)
        .execute(pool)
        .await
        .expect("OCI image");
    }
}

pub async fn seed_source(
    pool: &PgPool,
    repository: Uuid,
    receive: Uuid,
    commit: &str,
    config: &Value,
    source_hash: &agent_config::ConfigHash,
    normalized_hash: &agent_config::ConfigHash,
) {
    sqlx::query(
        "INSERT INTO git_receives (id, repository_id, principal, status)
         VALUES ($1, $2, 'manual-ui-test', 'accepted')",
    )
    .bind(receive)
    .bind(repository)
    .execute(pool)
    .await
    .expect("receive");
    sqlx::query(
        "INSERT INTO git_refs (repository_id, git_ref, commit_sha, updated_by_receive_id)
         VALUES ($1, $2, $3, $4)",
    )
    .bind(repository)
    .bind(format!("refs/heads/manual-{}", &commit[..8]))
    .bind(commit)
    .bind(receive)
    .execute(pool)
    .await
    .expect("Git ref");
    sqlx::query(
        "INSERT INTO agent_config_revisions
         (id, repository_id, receive_id, commit_sha, config_hash, normalized_config_hash,
          schema_version, status, config)
         VALUES ($1, $2, $3, $4, $5, $6, 2, 'valid', $7)",
    )
    .bind(Uuid::new_v4())
    .bind(repository)
    .bind(receive)
    .bind(commit)
    .bind(source_hash.as_str())
    .bind(normalized_hash.as_str())
    .bind(config)
    .execute(pool)
    .await
    .expect("agent configuration revision");
}

pub async fn seed_valid_ui(
    pool: &PgPool,
    repository: Uuid,
    receive: Uuid,
    commit: &str,
    hash: [u8; 32],
) {
    sqlx::query(
        "INSERT INTO ui_source_manifest_revisions
         (id, repository_id, receive_id, source_commit, manifest_path, entry_kind,
          manifest_oid, actual_size_bytes, source_sha256, status, requires_gateways,
          normalized_ui_config, normalized_ui_hash, diagnostics)
         VALUES ($1, $2, $3, $4, 'heph.ui.toml', 'blob', repeat('a', 40), 1,
                 $5, 'valid', false, '{}', $6, '[]')",
    )
    .bind(Uuid::new_v4())
    .bind(repository)
    .bind(receive)
    .bind(commit)
    .bind([3_u8; 32].as_slice())
    .bind(hash.as_slice())
    .execute(pool)
    .await
    .expect("valid UI capture");
}

pub async fn seed_invalid_ui(pool: &PgPool, repository: Uuid, receive: Uuid, commit: &str) {
    sqlx::query(
        "INSERT INTO ui_source_manifest_revisions
         (id, repository_id, receive_id, source_commit, manifest_path, entry_kind,
          manifest_oid, status, requires_gateways, diagnostics)
         VALUES ($1, $2, $3, $4, 'heph.ui.toml', 'blob', repeat('b', 40),
                 'invalid', false, '[{\"code\":\"invalid\"}]')",
    )
    .bind(Uuid::new_v4())
    .bind(repository)
    .bind(receive)
    .bind(commit)
    .execute(pool)
    .await
    .expect("invalid UI capture");
}
