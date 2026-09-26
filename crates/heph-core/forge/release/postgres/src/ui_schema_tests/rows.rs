use super::fixtures::ReleaseFixture;
use sqlx::PgPool;

pub async fn seed_valid_ui_rows(pool: &PgPool, fixture: &ReleaseFixture) {
    sqlx::query(
        "INSERT INTO release_ui_descriptors
         (release_id, ui_key, scope, label, icon, presentation, route_base,
          entrypoint, ui_kit_version, cache, content_kind)
         VALUES ($1, 'docs', 'global', 'Docs', 'book', 'iframe',
                 'docs', 'index.html', 1, 'no_store', 'static'),
                ($1, 'service', 'global', 'Service', 'app', 'full_page',
                 'service', 'index.html', 1, 'no_store', 'managed_service')",
    )
    .bind(fixture.release)
    .execute(pool)
    .await
    .expect("seed UI descriptors");
    sqlx::query(
        "INSERT INTO release_ui_static_files
         (release_id, ui_key, route, artifact_id, artifact_kind, artifact_media_type)
         VALUES ($1, 'docs', 'index.html', $2, 'file', 'text/html')",
    )
    .bind(fixture.release)
    .bind(fixture.artifact)
    .execute(pool)
    .await
    .expect("seed static UI file");
    sqlx::query(
        "INSERT INTO release_ui_managed_services
         (release_id, ui_key, gateway_name, route, release_agent_id)
         VALUES ($1, 'service', 'ui-service', '/service', $2)",
    )
    .bind(fixture.release)
    .bind(fixture.release_agent)
    .execute(pool)
    .await
    .expect("seed managed UI service");
    sqlx::query(
        "INSERT INTO release_ui_api_bindings
         (release_id, ui_key, api_key, gateway_name, method, route, release_agent_id)
         VALUES ($1, 'service', 'health', 'ui-service', 'GET', '/service/health', $2)",
    )
    .bind(fixture.release)
    .bind(fixture.release_agent)
    .execute(pool)
    .await
    .expect("seed UI API binding");
}

pub async fn assert_valid_rows(pool: &PgPool, fixture: &ReleaseFixture) {
    let counts: (i64, i64, i64, i64, i64) = sqlx::query_as(
        "SELECT
            (SELECT count(*) FROM release_ui_source_snapshots WHERE release_id = $1),
            (SELECT count(*) FROM release_ui_descriptors WHERE release_id = $1),
            (SELECT count(*) FROM release_ui_static_files WHERE release_id = $1),
            (SELECT count(*) FROM release_ui_managed_services WHERE release_id = $1),
            (SELECT count(*) FROM release_ui_api_bindings WHERE release_id = $1)",
    )
    .bind(fixture.release)
    .fetch_one(pool)
    .await
    .expect("read valid UI rows");
    assert_eq!(counts, (1, 2, 1, 1, 1));
}
