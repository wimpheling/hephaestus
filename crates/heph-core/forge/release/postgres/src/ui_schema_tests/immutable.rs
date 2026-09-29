use super::{fixtures::ReleaseFixture, support::assert_sqlstate};
use sqlx::PgPool;

pub async fn assert_immutable_rows(pool: &PgPool, fixture: &ReleaseFixture) {
    let mutations = [
        sqlx::query("UPDATE release_ui_source_snapshots SET created_at = now() WHERE release_id = $1")
            .bind(fixture.release),
        sqlx::query("UPDATE release_ui_descriptors SET label = 'Changed' WHERE release_id = $1 AND ui_key = 'docs'")
            .bind(fixture.release),
        sqlx::query("UPDATE release_ui_static_files SET route = 'changed.html' WHERE release_id = $1 AND ui_key = 'docs' AND route = 'index.html'")
            .bind(fixture.release),
        sqlx::query("UPDATE release_ui_managed_services SET route = '/changed' WHERE release_id = $1 AND ui_key = 'service'")
            .bind(fixture.release),
        sqlx::query("UPDATE release_ui_api_bindings SET method = 'POST' WHERE release_id = $1 AND ui_key = 'service' AND api_key = 'health'")
            .bind(fixture.release),
    ];
    for mutation in mutations {
        assert_sqlstate(mutation.execute(pool).await, "23000");
    }
    let deletes = [
        sqlx::query("DELETE FROM release_ui_source_snapshots WHERE release_id = $1")
            .bind(fixture.release),
        sqlx::query("DELETE FROM release_ui_descriptors WHERE release_id = $1 AND ui_key = 'docs'")
            .bind(fixture.release),
        sqlx::query("DELETE FROM release_ui_static_files WHERE release_id = $1 AND ui_key = 'docs' AND route = 'index.html'")
            .bind(fixture.release),
        sqlx::query("DELETE FROM release_ui_managed_services WHERE release_id = $1 AND ui_key = 'service'")
            .bind(fixture.release),
        sqlx::query("DELETE FROM release_ui_api_bindings WHERE release_id = $1 AND ui_key = 'service' AND api_key = 'health'")
            .bind(fixture.release),
    ];
    for deletion in deletes {
        assert_sqlstate(deletion.execute(pool).await, "23000");
    }
}

pub async fn assert_insert_guard(pool: &PgPool, fixture: &ReleaseFixture, state: &str) {
    let source = sqlx::query(
        "INSERT INTO release_ui_source_snapshots
         (release_id, build_request_id, source_manifest_revision_id)
         VALUES ($1, $2, $3)",
    )
    .bind(fixture.release_without_ui)
    .bind(fixture.build_without_ui)
    .bind(fixture.source_revision_without_ui)
    .execute(pool)
    .await;
    assert_sqlstate(source, "23000");

    let descriptor = sqlx::query(
        "INSERT INTO release_ui_descriptors
         (release_id, ui_key, scope, label, icon, presentation, route_base,
          entrypoint, ui_kit_version, cache, content_kind)
         VALUES ($1, 'after-state', 'global', 'After state', 'app', 'iframe',
                 'after-state', 'index.html', 1, 'no_store', 'static')",
    )
    .bind(fixture.release)
    .execute(pool)
    .await;
    assert_sqlstate(descriptor, "23000");

    let static_file = sqlx::query(
        "INSERT INTO release_ui_static_files
         (release_id, ui_key, route, artifact_id, artifact_kind, artifact_media_type)
         VALUES ($1, 'docs', $2, $3, 'file', 'text/html')",
    )
    .bind(fixture.release)
    .bind(format!("after-{state}.html"))
    .bind(fixture.artifact)
    .execute(pool)
    .await;
    assert_sqlstate(static_file, "23000");

    let managed = sqlx::query(
        "INSERT INTO release_ui_managed_services
         (release_id, ui_key, gateway_name, route, release_agent_id)
         VALUES ($1, 'service', 'ui-service', $2, $3)",
    )
    .bind(fixture.release)
    .bind(format!("/after-{state}"))
    .bind(fixture.release_agent)
    .execute(pool)
    .await;
    assert_sqlstate(managed, "23000");

    let api = sqlx::query(
        "INSERT INTO release_ui_api_bindings
         (release_id, ui_key, api_key, gateway_name, method, route, release_agent_id)
         VALUES ($1, 'service', $2, 'ui-service', 'GET', '/after-state', $3)",
    )
    .bind(fixture.release)
    .bind(format!("after-{state}"))
    .bind(fixture.release_agent)
    .execute(pool)
    .await;
    assert_sqlstate(api, "23000");
}
