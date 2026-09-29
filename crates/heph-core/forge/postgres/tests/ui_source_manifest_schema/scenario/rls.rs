use serde_json::json;
use uuid::Uuid;

use super::Context;
use crate::support::{VALID_COMMIT, app_pool, assert_sqlstate, commit};

pub async fn run(context: &Context, database_url: &str) {
    let fixture = context.fixture;
    let app = app_pool(database_url).await;
    sqlx::query("SELECT set_config('hephaestus.actor_id', $1, false)")
        .bind(fixture.outsider_user.to_string())
        .execute(&app)
        .await
        .expect("set application actor");
    let app_visible: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ui_source_manifest_revisions
         WHERE repository_id = $1",
    )
    .bind(fixture.public_repository)
    .fetch_one(&app)
    .await
    .expect("public repository source read");
    assert_eq!(app_visible, 3, "public repository rows are readable");
    let private_visible: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ui_source_manifest_revisions
         WHERE repository_id = $1",
    )
    .bind(fixture.private_repository)
    .fetch_one(&app)
    .await
    .expect("private repository source read");
    assert_eq!(private_visible, 0, "private repository rows stay isolated");

    let unauthorized_insert = sqlx::query(
        "INSERT INTO ui_source_manifest_revisions
         (id, repository_id, receive_id, source_commit, entry_kind, manifest_oid,
          actual_size_bytes, source_sha256, status, normalized_ui_config,
          normalized_ui_hash)
         VALUES ($1, $2, $3, $4, 'blob', $5, 3, $6, 'valid', $7, $8)",
    )
    .bind(Uuid::new_v4())
    .bind(fixture.public_repository)
    .bind(fixture.public_receive)
    .bind(commit("f"))
    .bind(commit("d"))
    .bind([3_u8; 32].as_slice())
    .bind(json!({"version": 1}))
    .bind([4_u8; 32].as_slice())
    .execute(&app)
    .await;
    assert!(
        unauthorized_insert.is_err(),
        "public read must not grant write"
    );
    assert_sqlstate(unauthorized_insert, "42501");

    let unauthorized_link = sqlx::query(
        "INSERT INTO build_request_ui_source_manifests
         (build_request_id, repository_id, source_commit,
          source_manifest_revision_id)
         VALUES ($1, $2, $3, $4)",
    )
    .bind(fixture.private_build)
    .bind(fixture.private_repository)
    .bind(commit(VALID_COMMIT))
    .bind(fixture.private_revision)
    .execute(&app)
    .await;
    assert!(
        unauthorized_link.is_err(),
        "private link requires build access"
    );
    assert_sqlstate(unauthorized_link, "42501");
}
