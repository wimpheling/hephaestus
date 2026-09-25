use super::ui_installation_seed::SeedData;
use serde_json::json;
use sqlx::PgPool;

pub(crate) async fn seed_base_rows(pool: &PgPool, data: &SeedData) {
    sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, $2)")
        .bind(data.user_id)
        .bind("UI RPC owner")
        .execute(pool)
        .await
        .expect("seed UI RPC user");
    sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, $2)")
        .bind(data.second_user_id)
        .bind("UI RPC second admin")
        .execute(pool)
        .await
        .expect("seed second UI RPC user");
    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, $2)")
        .bind(data.organization_id)
        .bind("UI RPC organization")
        .execute(pool)
        .await
        .expect("seed UI RPC organization");
    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, $2)")
        .bind(data.foreign_organization_id)
        .bind("UI RPC foreign organization")
        .execute(pool)
        .await
        .expect("seed foreign UI RPC organization");
    sqlx::query(
        "INSERT INTO organization_members (organization_id, user_id, role)
             VALUES ($1, $2, 'owner')",
    )
    .bind(data.organization_id)
    .bind(data.user_id)
    .execute(pool)
    .await
    .expect("seed UI RPC organization owner");
    sqlx::query(
        "INSERT INTO organization_members (organization_id, user_id, role)
             VALUES ($1, $2, 'admin')",
    )
    .bind(data.organization_id)
    .bind(data.second_user_id)
    .execute(pool)
    .await
    .expect("seed second UI RPC organization admin");
    sqlx::query(
        "INSERT INTO organization_members (organization_id, user_id, role)
             VALUES ($1, $2, 'admin')",
    )
    .bind(data.foreign_organization_id)
    .bind(data.user_id)
    .execute(pool)
    .await
    .expect("seed cross-tenant UI RPC admin");
    sqlx::query("INSERT INTO projects (id, organization_id, name) VALUES ($1, $2, $3)")
        .bind(data.project_id)
        .bind(data.organization_id)
        .bind("ui-rpc-project")
        .execute(pool)
        .await
        .expect("seed UI RPC project");
    sqlx::query("INSERT INTO project_maintainers (project_id, user_id) VALUES ($1, $2)")
        .bind(data.project_id)
        .bind(data.user_id)
        .execute(pool)
        .await
        .expect("seed UI RPC project maintainer");
    sqlx::query("INSERT INTO project_maintainers (project_id, user_id) VALUES ($1, $2)")
        .bind(data.project_id)
        .bind(data.second_user_id)
        .execute(pool)
        .await
        .expect("seed second UI RPC project maintainer");
    sqlx::query(
        "INSERT INTO repositories (id, project_id, name, default_branch, is_public)
             VALUES ($1, $2, $3, 'refs/heads/main', false)",
    )
    .bind(data.repository_id)
    .bind(data.project_id)
    .bind("ui-rpc-repository")
    .execute(pool)
    .await
    .expect("seed UI RPC repository");
    sqlx::query(
        "INSERT INTO git_receives (id, repository_id, actor_id, principal, status, accepted_at)
             VALUES ($1, $2, $3, 'ui-rpc-test', 'accepted', now())",
    )
    .bind(data.receive_id)
    .bind(data.repository_id)
    .bind(data.user_id)
    .execute(pool)
    .await
    .expect("seed UI RPC receive");
    sqlx::query(
        "INSERT INTO build_requests
             (id, repository_id, source_commit, source_ref, origin_receive_id,
              build_definition_hash, state, created_by)
             VALUES ($1, $2, $3, 'refs/heads/main', $4, $5, 'succeeded', $6)",
    )
    .bind(data.build_id)
    .bind(data.repository_id)
    .bind(&data.commit)
    .bind(data.receive_id)
    .bind([1_u8; 32].as_slice())
    .bind(data.user_id)
    .execute(pool)
    .await
    .expect("seed UI RPC build");
    sqlx::query(
        "INSERT INTO ui_source_manifest_revisions
             (id, repository_id, receive_id, source_commit, entry_kind, manifest_oid,
              actual_size_bytes, source_sha256, status, requires_gateways,
              normalized_ui_config, normalized_ui_hash, diagnostics)
             VALUES ($1, $2, $3, $4, 'blob', $5, 2, $6, 'valid', false, $7, $8, '[]')",
    )
    .bind(data.source_revision_id)
    .bind(data.repository_id)
    .bind(data.receive_id)
    .bind(&data.commit)
    .bind(&data.commit)
    .bind([2_u8; 32].as_slice())
    .bind(json!({"uis": []}))
    .bind([3_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("seed UI source revision");
    sqlx::query(
        "INSERT INTO build_request_ui_source_manifests
             (build_request_id, repository_id, source_commit, source_manifest_revision_id)
             VALUES ($1, $2, $3, $4)",
    )
    .bind(data.build_id)
    .bind(data.repository_id)
    .bind(&data.commit)
    .bind(data.source_revision_id)
    .execute(pool)
    .await
    .expect("seed UI source capture");
}
