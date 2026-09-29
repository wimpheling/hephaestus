use super::{fixtures::ReleaseFixture, support::assert_sqlstate};
use sqlx::PgPool;

// Keep the negative SQL assertions together to show the complete constraint set.
#[allow(clippy::too_many_lines)]
pub async fn assert_shape_constraints(pool: &PgPool, fixture: &ReleaseFixture) {
    let cross_artifact = sqlx::query(
        "INSERT INTO release_ui_static_files
         (release_id, ui_key, route, artifact_id, artifact_kind, artifact_media_type)
         VALUES ($1, 'docs', 'cross-artifact', $2, 'file', 'text/html')",
    )
    .bind(fixture.release)
    .bind(fixture.foreign_artifact)
    .execute(pool)
    .await;
    assert_sqlstate(cross_artifact, "23503");

    sqlx::query(
        "INSERT INTO release_ui_descriptors
         (release_id, ui_key, scope, label, icon, presentation, route_base,
          entrypoint, ui_kit_version, cache, content_kind)
         VALUES ($1, 'service-cross-agent', 'global', 'Cross agent', 'app',
                 'full_page', 'service-cross-agent', 'index.html', 1,
                 'no_store', 'managed_service')",
    )
    .bind(fixture.release)
    .execute(pool)
    .await
    .expect("seed cross-agent managed descriptor");
    let cross_agent = sqlx::query(
        "INSERT INTO release_ui_managed_services
         (release_id, ui_key, gateway_name, route, release_agent_id)
         VALUES ($1, 'service-cross-agent', 'ui-service', '/cross-agent', $2)",
    )
    .bind(fixture.release)
    .bind(fixture.foreign_agent)
    .execute(pool)
    .await;
    assert_sqlstate(cross_agent, "23503");

    let cross_build = sqlx::query(
        "INSERT INTO release_ui_source_snapshots
         (release_id, build_request_id, source_manifest_revision_id)
         VALUES ($1, $2, $3)",
    )
    .bind(fixture.release_without_ui)
    .bind(fixture.build)
    .bind(fixture.source_revision)
    .execute(pool)
    .await;
    assert_sqlstate(cross_build, "23503");

    let no_snapshot = sqlx::query(
        "INSERT INTO release_ui_descriptors
         (release_id, ui_key, scope, label, icon, presentation, route_base,
          entrypoint, ui_kit_version, cache, content_kind)
         VALUES ($1, 'missing-source', 'global', 'Missing', 'app', 'iframe',
                 'missing', 'index.html', 1, 'no_store', 'static')",
    )
    .bind(fixture.release_without_ui)
    .execute(pool)
    .await;
    assert_sqlstate(no_snapshot, "23503");

    for (key, route_base) in [
        ("double-slash", "bad//path"),
        ("dot-segment", "bad/../path"),
    ] {
        let result = sqlx::query(
            "INSERT INTO release_ui_descriptors
             (release_id, ui_key, scope, label, icon, presentation, route_base,
              entrypoint, ui_kit_version, cache, content_kind)
             VALUES ($1, $2, 'global', 'Bad', 'app', 'iframe', $3,
                     'index.html', 1, 'no_store', 'static')",
        )
        .bind(fixture.release)
        .bind(key)
        .bind(route_base)
        .execute(pool)
        .await;
        assert_sqlstate(result, "23514");
    }
    for (route, key) in [("//service", "double-route"), ("/../service", "dot-route")] {
        let result = sqlx::query(
            "INSERT INTO release_ui_api_bindings
             (release_id, ui_key, api_key, gateway_name, method, route, release_agent_id)
             VALUES ($1, 'service', $2, 'ui-service', 'GET', $3, $4)",
        )
        .bind(fixture.release)
        .bind(key)
        .bind(route)
        .bind(fixture.release_agent)
        .execute(pool)
        .await;
        assert_sqlstate(result, "23514");
    }

    let wrong_kind = sqlx::query(
        "INSERT INTO release_ui_static_files
         (release_id, ui_key, route, artifact_id, artifact_kind, artifact_media_type)
         VALUES ($1, 'docs', 'wrong-kind', $2, 'executable', 'text/html')",
    )
    .bind(fixture.release)
    .bind(fixture.artifact)
    .execute(pool)
    .await;
    assert_sqlstate(wrong_kind, "23514");
    let wrong_media = sqlx::query(
        "INSERT INTO release_ui_static_files
         (release_id, ui_key, route, artifact_id, artifact_kind, artifact_media_type)
         VALUES ($1, 'docs', 'wrong-media', $2, 'file', 'text/css')",
    )
    .bind(fixture.release)
    .bind(fixture.artifact)
    .execute(pool)
    .await;
    assert_sqlstate(wrong_media, "23503");
    let wrong_static_kind = sqlx::query(
        "INSERT INTO release_ui_static_files
         (release_id, ui_key, route, content_kind, artifact_id, artifact_kind, artifact_media_type)
         VALUES ($1, 'docs', 'wrong-content', 'managed_service', $2, 'file', 'text/html')",
    )
    .bind(fixture.release)
    .bind(fixture.artifact)
    .execute(pool)
    .await;
    assert_sqlstate(wrong_static_kind, "23514");
    let wrong_managed_kind = sqlx::query(
        "INSERT INTO release_ui_managed_services
         (release_id, ui_key, content_kind, gateway_name, route, release_agent_id)
         VALUES ($1, 'service', 'static', 'ui-service', '/wrong-content', $2)",
    )
    .bind(fixture.release)
    .bind(fixture.release_agent)
    .execute(pool)
    .await;
    assert_sqlstate(wrong_managed_kind, "23514");
}
