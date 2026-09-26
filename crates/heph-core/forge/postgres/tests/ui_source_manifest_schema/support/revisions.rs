use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use super::{Fixture, VALID_COMMIT, commit};

pub async fn insert_valid_revision(pool: &PgPool, repository: Uuid, receive: Uuid, revision: Uuid) {
    sqlx::query(
        "INSERT INTO ui_source_manifest_revisions
         (id, repository_id, receive_id, source_commit, entry_kind, manifest_oid,
          actual_size_bytes, source_sha256, status, normalized_ui_config,
          normalized_ui_hash)
         VALUES ($1, $2, $3, $4, 'blob', $5, 32, $6, 'valid', $7, $8)",
    )
    .bind(revision)
    .bind(repository)
    .bind(receive)
    .bind(commit(VALID_COMMIT))
    .bind(commit("c"))
    .bind([1_u8; 32].as_slice())
    .bind(json!({"version": 1, "uis": []}))
    .bind([2_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("valid source revision");
}

pub async fn insert_invalid_oversized(pool: &PgPool, fixture: &Fixture) {
    sqlx::query(
        "INSERT INTO ui_source_manifest_revisions
         (id, repository_id, receive_id, source_commit, entry_kind, manifest_oid,
          actual_size_bytes, status, diagnostics)
         VALUES ($1, $2, $3, $4, 'blob', $5, 262145, 'invalid', $6)",
    )
    .bind(fixture.oversized_revision)
    .bind(fixture.public_repository)
    .bind(fixture.public_receive)
    .bind(commit("c"))
    .bind(commit("e"))
    .bind(json!([{"code": "ui_manifest_too_large"}]))
    .execute(pool)
    .await
    .expect("oversized invalid source revision");
}

pub async fn insert_invalid_non_blob(pool: &PgPool, fixture: &Fixture) {
    sqlx::query(
        "INSERT INTO ui_source_manifest_revisions
         (id, repository_id, receive_id, source_commit, entry_kind, manifest_oid,
          status, diagnostics)
         VALUES ($1, $2, $3, $4, 'symlink', $5, 'invalid', $6)",
    )
    .bind(Uuid::new_v4())
    .bind(fixture.public_repository)
    .bind(fixture.public_receive)
    .bind(commit("d"))
    .bind(commit("e"))
    .bind(json!([{"code": "ui_manifest_not_regular_blob"}]))
    .execute(pool)
    .await
    .expect("non-blob invalid source revision");
}
