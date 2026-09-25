use super::ui_installation_seed::SeedData;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

pub(crate) async fn seed_release_rows(pool: &PgPool, data: &SeedData) {
    sqlx::query(
        "INSERT INTO releases
             (id, repository_id, version, source_commit, source_ref, build_request_id,
              build_definition_hash, configuration, configuration_hash, manifest_hash,
              state, publication_actor_id)
             VALUES ($1, $2, 'ui-rpc-v1', $3, 'refs/heads/main', $4, $5, $6, $7, $8,
                     'draft', $9)",
    )
    .bind(data.release_id)
    .bind(data.repository_id)
    .bind(&data.commit)
    .bind(data.build_id)
    .bind([4_u8; 32].as_slice())
    .bind(json!({"ui": true}))
    .bind([5_u8; 32].as_slice())
    .bind([6_u8; 32].as_slice())
    .bind(data.user_id)
    .execute(pool)
    .await
    .expect("seed UI release");
    for (id, version) in [
        (data.global_release_id, "ui-rpc-global"),
        (data.repository_release_id, "ui-rpc-repository"),
    ] {
        sqlx::query(
            "INSERT INTO releases
                 (id, repository_id, version, source_commit, source_ref, build_request_id,
                  build_definition_hash, configuration, configuration_hash, manifest_hash,
                  state, publication_actor_id)
                 VALUES ($1, $2, $3, $4, 'refs/heads/main', $5, $6, $7, $8, $9,
                         'draft', $10)",
        )
        .bind(id)
        .bind(data.repository_id)
        .bind(version)
        .bind(&data.commit)
        .bind(data.build_id)
        .bind([4_u8; 32].as_slice())
        .bind(json!({"ui": true}))
        .bind([5_u8; 32].as_slice())
        .bind([6_u8; 32].as_slice())
        .bind(data.user_id)
        .execute(pool)
        .await
        .expect("seed scoped UI release");
    }
    sqlx::query(
        "INSERT INTO release_ui_source_snapshots
             (release_id, build_request_id, source_manifest_revision_id)
             VALUES ($1, $2, $3)",
    )
    .bind(data.release_id)
    .bind(data.build_id)
    .bind(data.source_revision_id)
    .execute(pool)
    .await
    .expect("seed UI release source snapshot");
    for release in [data.global_release_id, data.repository_release_id] {
        sqlx::query(
            "INSERT INTO release_ui_source_snapshots
                 (release_id, build_request_id, source_manifest_revision_id)
                 VALUES ($1, $2, $3)",
        )
        .bind(release)
        .bind(data.build_id)
        .bind(data.source_revision_id)
        .execute(pool)
        .await
        .expect("seed scoped UI release source snapshot");
    }
    sqlx::query(
        "INSERT INTO release_ui_descriptors
             (release_id, ui_key, scope, label, icon, presentation, route_base,
              entrypoint, ui_kit_version, cache, content_kind)
             VALUES ($1, 'assistant', 'project', 'Assistant', 'app', 'full_page',
                     'assistant', 'index.html', 1, 'no_store', 'static')",
    )
    .bind(data.release_id)
    .execute(pool)
    .await
    .expect("seed UI descriptor");
    sqlx::query(
        "INSERT INTO release_ui_descriptors
             (release_id, ui_key, scope, label, icon, presentation, route_base,
              entrypoint, ui_kit_version, cache, content_kind)
             VALUES ($1, 'assistant', 'global', 'Assistant', 'app', 'full_page',
                     'assistant', 'index.html', 1, 'no_store', 'static')",
    )
    .bind(data.global_release_id)
    .execute(pool)
    .await
    .expect("seed global UI descriptor");
    sqlx::query(
        "INSERT INTO release_ui_descriptors
             (release_id, ui_key, scope, label, icon, presentation, route_base,
              entrypoint, ui_kit_version, cache, content_kind)
             VALUES ($1, 'assistant', 'repository', 'Assistant', 'app', 'full_page',
                     'assistant', 'index.html', 1, 'no_store', 'static')",
    )
    .bind(data.repository_release_id)
    .execute(pool)
    .await
    .expect("seed repository UI descriptor");
    sqlx::query(
        "INSERT INTO release_ui_descriptors
             (release_id, ui_key, scope, label, icon, presentation, route_base,
              entrypoint, ui_kit_version, cache, content_kind)
             VALUES ($1, 'console', 'global', 'Console', 'app', 'full_page',
                     'console', 'index.html', 1, 'no_store', 'static')",
    )
    .bind(data.global_release_id)
    .execute(pool)
    .await
    .expect("seed second global UI descriptor");
    sqlx::query(
        "INSERT INTO release_artifacts
             (id, release_id, path, kind, mode, content_hash, size_bytes, media_type, storage_key)
             VALUES ($1, $2, 'index.html', 'file', 420, $3, 19, 'text/html', $4)",
    )
    .bind(data.artifact_id)
    .bind(data.release_id)
    .bind([7_u8; 32].as_slice())
    .bind(Uuid::new_v4())
    .execute(pool)
    .await
    .expect("seed UI artifact");
    for (artifact, release) in [
        (data.global_artifact_id, data.global_release_id),
        (data.repository_artifact_id, data.repository_release_id),
    ] {
        sqlx::query(
            "INSERT INTO release_artifacts
                 (id, release_id, path, kind, mode, content_hash, size_bytes,
                  media_type, storage_key)
                 VALUES ($1, $2, 'index.html', 'file', 420, $3, 19,
                         'text/html', $4)",
        )
        .bind(artifact)
        .bind(release)
        .bind([7_u8; 32].as_slice())
        .bind(Uuid::new_v4())
        .execute(pool)
        .await
        .expect("seed scoped UI artifact");
    }
    sqlx::query(
        "INSERT INTO release_ui_static_files
             (release_id, ui_key, route, artifact_id, artifact_kind, artifact_media_type)
             VALUES ($1, 'assistant', 'index.html', $2, 'file', 'text/html')",
    )
    .bind(data.release_id)
    .bind(data.artifact_id)
    .execute(pool)
    .await
    .expect("seed UI static file");
    sqlx::query(
        "INSERT INTO release_ui_static_files
             (release_id, ui_key, route, artifact_id, artifact_kind, artifact_media_type)
             VALUES ($1, 'assistant', 'index.html', $2, 'file', 'text/html')",
    )
    .bind(data.global_release_id)
    .bind(data.global_artifact_id)
    .execute(pool)
    .await
    .expect("seed global assistant UI static file");
    sqlx::query(
        "INSERT INTO release_ui_static_files
             (release_id, ui_key, route, artifact_id, artifact_kind, artifact_media_type)
             VALUES ($1, 'console', 'index.html', $2, 'file', 'text/html')",
    )
    .bind(data.global_release_id)
    .bind(data.global_artifact_id)
    .execute(pool)
    .await
    .expect("seed global UI static file");
    sqlx::query(
        "INSERT INTO release_ui_static_files
             (release_id, ui_key, route, artifact_id, artifact_kind, artifact_media_type)
             VALUES ($1, 'assistant', 'index.html', $2, 'file', 'text/html')",
    )
    .bind(data.repository_release_id)
    .bind(data.repository_artifact_id)
    .execute(pool)
    .await
    .expect("seed repository UI static file");
    sqlx::query(
        "UPDATE releases SET state = 'published', published_at = now()
             WHERE id = ANY($1)",
    )
    .bind(vec![
        data.release_id,
        data.global_release_id,
        data.repository_release_id,
    ])
    .execute(pool)
    .await
    .expect("publish UI releases");
}
