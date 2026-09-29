use sqlx::PgPool;
use std::time::Duration;
use tokio::time::timeout;
use uuid::Uuid;

pub async fn seed_draft_global_release(
    pool: &PgPool,
    template_release: Uuid,
    release_id: Uuid,
    ui_key: &str,
) {
    sqlx::query(
        "INSERT INTO releases
         (id, repository_id, version, source_commit, source_ref, build_request_id,
          build_definition_hash, configuration, configuration_hash, manifest_hash, state)
         SELECT $1, repository_id, $3, source_commit, source_ref, build_request_id,
                build_definition_hash, configuration, configuration_hash, manifest_hash,
                'draft'
         FROM releases
         WHERE id = $2",
    )
    .bind(release_id)
    .bind(template_release)
    .bind(format!("ui-barrier-{release_id}"))
    .execute(pool)
    .await
    .expect("seed draft source release");
    sqlx::query(
        "INSERT INTO release_ui_source_snapshots
         (release_id, build_request_id, source_manifest_revision_id)
         SELECT $1, build_request_id, source_manifest_revision_id
         FROM release_ui_source_snapshots
         WHERE release_id = $2",
    )
    .bind(release_id)
    .bind(template_release)
    .execute(pool)
    .await
    .expect("seed draft source snapshot");
    sqlx::query(
        "INSERT INTO release_ui_descriptors
         (release_id, ui_key, scope, label, icon, presentation, route_base,
          entrypoint, ui_kit_version, cache, content_kind)
         VALUES ($1, $2, 'global', 'Release Barrier UI', 'app', 'iframe', $2,
                 'index.html', 1, 'no_store', 'static')",
    )
    .bind(release_id)
    .bind(ui_key)
    .execute(pool)
    .await
    .expect("seed draft global descriptor");
}

pub async fn seed_global_installation(
    pool: &PgPool,
    organization_id: Uuid,
    installation_id: Uuid,
    generation_id: Uuid,
    release_id: Uuid,
    ui_key: &str,
) {
    let mut tx = pool.begin().await.expect("begin global installation seed");
    sqlx::query(
        "INSERT INTO ui_installations
         (id, organization_id, project_id, repository_id, scope, ui_key,
          lifecycle, current_generation_id, created_by)
         VALUES ($1, $2, NULL, NULL, 'global', $3, 'enabled', $4,
                 (SELECT user_id FROM organization_members
                  WHERE organization_id = $2 ORDER BY user_id LIMIT 1))",
    )
    .bind(installation_id)
    .bind(organization_id)
    .bind(ui_key)
    .bind(generation_id)
    .execute(&mut *tx)
    .await
    .expect("seed global installation");
    sqlx::query(
        "INSERT INTO ui_installation_generations
         (id, installation_id, generation_no, release_id, ui_key, ui_scope)
         VALUES ($1, $2, 1, $3, $4, 'global')",
    )
    .bind(generation_id)
    .bind(installation_id)
    .bind(release_id)
    .bind(ui_key)
    .execute(&mut *tx)
    .await
    .expect("seed global generation");
    tx.commit().await.expect("commit global installation seed");
}

pub async fn wait_until_blocked(pool: &PgPool, pid: i32, expected_blocker: i32) {
    timeout(Duration::from_secs(5), async {
        loop {
            let blockers: Vec<i32> = sqlx::query_scalar("SELECT pg_blocking_pids($1)")
                .bind(pid)
                .fetch_one(pool)
                .await
                .expect("read blocking PostgreSQL PIDs");
            if blockers.contains(&expected_blocker) {
                return;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("generation transaction reached the source-parent lock barrier");
}
