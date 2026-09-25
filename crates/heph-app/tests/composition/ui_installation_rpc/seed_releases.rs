use super::ui_installation_seed::SeedData;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

pub(super) async fn seed_release_rows(pool: &PgPool, data: &SeedData) {
    seed_release_records(pool, data).await;
    seed_release_snapshots(pool, data).await;
    seed_release_descriptors(pool, data).await;
    seed_release_artifacts(pool, data).await;
}

async fn seed_release_records(pool: &PgPool, data: &SeedData) {
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
}

async fn seed_release_snapshots(pool: &PgPool, data: &SeedData) {
    for release in [
        data.release_id,
        data.global_release_id,
        data.repository_release_id,
    ] {
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
        .expect("seed UI release source snapshot");
    }
}

async fn seed_release_descriptors(pool: &PgPool, data: &SeedData) {
    for (release, ui_key, scope, label) in [
        (data.release_id, "assistant", "project", "Assistant"),
        (data.global_release_id, "assistant", "global", "Assistant"),
        (
            data.repository_release_id,
            "assistant",
            "repository",
            "Assistant",
        ),
        (data.global_release_id, "console", "global", "Console"),
    ] {
        sqlx::query(
            "INSERT INTO release_ui_descriptors
                 (release_id, ui_key, scope, label, icon, presentation, route_base,
                  entrypoint, ui_kit_version, cache, content_kind)
                 VALUES ($1, $2, $3, $4, 'app', 'full_page', $5, 'index.html',
                         1, 'no_store', 'static')",
        )
        .bind(release)
        .bind(ui_key)
        .bind(scope)
        .bind(label)
        .bind(ui_key)
        .execute(pool)
        .await
        .expect("seed UI descriptor");
    }
}

async fn seed_release_artifacts(pool: &PgPool, data: &SeedData) {
    for (artifact, release) in [
        (data.artifact_id, data.release_id),
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
        .expect("seed UI artifact");
    }
    for (release, ui_key, artifact) in [
        (data.release_id, "assistant", data.artifact_id),
        (data.global_release_id, "assistant", data.global_artifact_id),
        (data.global_release_id, "console", data.global_artifact_id),
        (
            data.repository_release_id,
            "assistant",
            data.repository_artifact_id,
        ),
    ] {
        sqlx::query(
            "INSERT INTO release_ui_static_files
                 (release_id, ui_key, route, artifact_id, artifact_kind, artifact_media_type)
                 VALUES ($1, $2, 'index.html', $3, 'file', 'text/html')",
        )
        .bind(release)
        .bind(ui_key)
        .bind(artifact)
        .execute(pool)
        .await
        .expect("seed UI static file");
    }
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
