use super::*;
use crate::fixture::FixtureSeedIds;

// Keep the SQL publication-parent seed together so its FK order is reviewable.
#[allow(clippy::too_many_lines)]
pub async fn seed_identity_source(worker: &PgPool, ids: &FixtureSeedIds) {
    sqlx::query(
    "INSERT INTO users (id, display_name) VALUES ($1, 'UI browser actor'), ($2, 'UI browser outsider')",
)
.bind(ids.actor)
.bind(ids.outsider)
.execute(worker)
.await
.expect("seed users");
    sqlx::query("SELECT set_config('hephaestus.actor_id', $1, false)")
        .bind(ids.actor.to_string())
        .execute(worker)
        .await
        .expect("set fixture actor");
    sqlx::query(
        "INSERT INTO organizations (id, name) VALUES ($1, 'UI browser org'), ($2, 'Other org')",
    )
    .bind(ids.organization)
    .bind(ids.other_organization)
    .execute(worker)
    .await
    .expect("seed organizations");
    sqlx::query(
        "INSERT INTO organization_members (organization_id, user_id, role)
     VALUES ($1, $2, 'owner')",
    )
    .bind(ids.organization)
    .bind(ids.actor)
    .execute(worker)
    .await
    .expect("seed organization owner");
    sqlx::query(
        "INSERT INTO projects (id, organization_id, name)
     VALUES ($1, $2, 'ui-browser-target'), ($3, $2, 'ui-browser-source')",
    )
    .bind(ids.project)
    .bind(ids.organization)
    .bind(ids.source_project)
    .execute(worker)
    .await
    .expect("seed project");
    sqlx::query(
        "INSERT INTO repositories (id, project_id, name) VALUES ($1, $2, 'ui-browser-repository')",
    )
    .bind(ids.repository)
    .bind(ids.source_project)
    .execute(worker)
    .await
    .expect("seed repository");
    sqlx::query(
        "INSERT INTO git_receives
     (id, repository_id, actor_id, principal, status, accepted_at)
     VALUES ($1, $2, $3, 'ui-browser-schema', 'accepted', now())",
    )
    .bind(ids.receive)
    .bind(ids.repository)
    .bind(ids.actor)
    .execute(worker)
    .await
    .expect("seed accepted receive");
    sqlx::query(
        "INSERT INTO build_requests
     (id, repository_id, source_commit, source_ref, origin_receive_id,
      build_definition_hash, state, created_by)
     VALUES ($1, $2, $3, 'refs/heads/main', $4, $5, 'succeeded', $6)",
    )
    .bind(ids.build)
    .bind(ids.repository)
    .bind("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
    .bind(ids.receive)
    .bind(vec![1_u8; 32])
    .bind(ids.actor)
    .execute(worker)
    .await
    .expect("seed build request");
    sqlx::query(
        "INSERT INTO ui_source_manifest_revisions
     (id, repository_id, receive_id, source_commit, entry_kind, manifest_oid,
      actual_size_bytes, source_sha256, status, normalized_ui_config, normalized_ui_hash)
     VALUES ($1, $2, $3, $4, 'blob', $5, 32, $6, 'valid', '{}'::jsonb, $7)",
    )
    .bind(ids.source_revision)
    .bind(ids.repository)
    .bind(ids.receive)
    .bind("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
    .bind("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
    .bind(vec![2_u8; 32])
    .bind(vec![3_u8; 32])
    .execute(worker)
    .await
    .expect("seed source manifest revision");
    sqlx::query(
        "INSERT INTO build_request_ui_source_manifests
     (build_request_id, repository_id, source_commit, source_manifest_revision_id)
     VALUES ($1, $2, $3, $4)",
    )
    .bind(ids.build)
    .bind(ids.repository)
    .bind("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
    .bind(ids.source_revision)
    .execute(worker)
    .await
    .expect("link source manifest revision");
}
