//! `CompleteBuild` publication and atomicity integration scenarios.

use super::*;

#[path = "complete_build/managed_api.rs"]
mod managed_api;

#[tokio::test]
#[serial]
// This real PostgreSQL case intentionally keeps both publication and legacy
// assertions together because they share the worker-role fixture setup.
#[allow(clippy::too_many_lines)]
async fn complete_build_persists_static_ui_and_preserves_legacy_no_ui_builds() {
    let Some(admin_pool) = pool().await else {
        return;
    };
    sqlx::migrate!("../../../../../migrations")
        .run(&admin_pool)
        .await
        .expect("apply application migrations");
    let Some(worker_pool) = worker_pool_named("heph-static-install-replay").await else {
        return;
    };

    let ui_fixture = seed(&admin_pool).await;
    let ui_manifest = br#"
version = 1

[[uis]]
key = "docs"
scope = "repository"
label = "Docs"
icon = "book"
presentation = "iframe"
route_base = "docs"
ui_kit_version = 1
cache = "no_store"

[uis.content]
kind = "static"
entrypoint = "index.html"

[[uis.content.files]]
route = "index.html"
artifact = "dist/index.html"
media_type = "text/html"
"#;
    let parsed_ui = parse_repository_uis(ui_manifest);
    let ui_config = parsed_ui.config.expect("valid UI fixture");
    let ui_hash = decode_test_hash(
        parsed_ui
            .normalized_hash
            .expect("normalized UI hash")
            .as_str(),
    );
    let (repository_id, receive_id, source_commit, config_json): (Uuid, Uuid, String, Value) =
        sqlx::query_as(
            "SELECT request.repository_id, request.origin_receive_id,
                    request.source_commit, revision.config
             FROM build_requests AS request
             JOIN agent_config_revisions AS revision
               ON revision.repository_id = request.repository_id
              AND revision.commit_sha = request.source_commit
             WHERE request.id = $1",
        )
        .bind(ui_fixture.build.as_uuid())
        .fetch_one(&admin_pool)
        .await
        .expect("load UI fixture identity");
    let config: AgentConfig = serde_json::from_value(config_json).expect("fixture config");
    let base_hash = agent_config::build_identity::base_build_definition_hash(
        config.build.as_ref().expect("build fixture"),
    )
    .expect("base build hash");
    let build_definition_hash =
        agent_config::build_identity::ui_build_definition_hash(base_hash, ui_hash, None);
    let source_manifest_revision_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO ui_source_manifest_revisions
         (id, repository_id, receive_id, source_commit, entry_kind, manifest_oid,
          actual_size_bytes, source_sha256, status, normalized_ui_config,
          normalized_ui_hash, diagnostics)
         VALUES ($1, $2, $3, $4, 'blob', $5, $6, $7, 'valid', $8, $9, '[]')",
    )
    .bind(source_manifest_revision_id)
    .bind(repository_id)
    .bind(receive_id)
    .bind(&source_commit)
    .bind("b".repeat(40))
    .bind(i64::try_from(ui_manifest.len()).expect("bounded fixture"))
    .bind([1_u8; 32].as_slice())
    .bind(serde_json::to_value(&ui_config).expect("UI JSON"))
    .bind(ui_hash.as_slice())
    .execute(&admin_pool)
    .await
    .expect("insert UI source capture");
    sqlx::query(
        "INSERT INTO build_request_ui_source_manifests
         (build_request_id, repository_id, source_commit,
          source_manifest_revision_id, source_status)
         VALUES ($1, $2, $3, $4, 'valid')",
    )
    .bind(ui_fixture.build.as_uuid())
    .bind(repository_id)
    .bind(&source_commit)
    .bind(source_manifest_revision_id)
    .execute(&admin_pool)
    .await
    .expect("link UI source capture");
    sqlx::query("UPDATE build_requests SET build_definition_hash = $2 WHERE id = $1")
        .bind(ui_fixture.build.as_uuid())
        .bind(build_definition_hash.as_slice())
        .execute(&admin_pool)
        .await
        .expect("store derived UI build identity");

    let ui_release_id = ReleaseId::new();
    let ui_release_agent_id = ReleaseAgentId::new();
    let ui_artifact_id = ReleaseArtifactId::new();
    let service = ReleaseService::new(worker_pool.clone(), Arc::new(PostgresMelangeAuthorizer));
    service
        .complete_build(CompleteBuild {
            command_key: key("complete-static-ui", ui_release_id.as_uuid()),
            build_request_id: ui_fixture.build,
            release_id: ui_release_id,
            version: ReleaseVersion::parse("static-ui-v1").expect("release version"),
            release_agent_id: ui_release_agent_id,
            artifacts: vec![ReleaseArtifactInput {
                id: ui_artifact_id,
                path: ArtifactPath::parse("dist/index.html").expect("artifact path"),
                kind: ArtifactKind::File,
                mode: 0o444,
                content_hash: ContentHash::digest(b"static-ui"),
                size_bytes: 9,
                media_type: String::from("text/html"),
                storage_key: Uuid::new_v4(),
            }],
        })
        .await
        .expect("complete static UI release");

    let stored_ui: (Uuid, Uuid, Uuid) = sqlx::query_as(
        "SELECT snapshot.source_manifest_revision_id,
                file.artifact_id, descriptor.release_id
         FROM release_ui_source_snapshots AS snapshot
         JOIN release_ui_static_files AS file
           ON file.release_id = snapshot.release_id
          AND file.ui_key = 'docs'
         JOIN release_ui_descriptors AS descriptor
           ON descriptor.release_id = file.release_id
          AND descriptor.ui_key = file.ui_key
         WHERE snapshot.release_id = $1",
    )
    .bind(ui_release_id.as_uuid())
    .fetch_one(&admin_pool)
    .await
    .expect("stored UI publication");
    assert_eq!(
        stored_ui,
        (
            source_manifest_revision_id,
            ui_artifact_id.as_uuid(),
            ui_release_id.as_uuid(),
        )
    );

    let legacy_fixture = seed(&admin_pool).await;
    sqlx::query(
        "UPDATE agent_config_revisions
            SET config = config - 'build'
          WHERE repository_id = (
                    SELECT repository_id FROM build_requests WHERE id = $1
                )
            AND commit_sha = repeat('a', 40)",
    )
    .bind(legacy_fixture.build.as_uuid())
    .execute(&admin_pool)
    .await
    .expect("make legacy configuration without build");
    let legacy_release_id = ReleaseId::new();
    service
        .complete_build(CompleteBuild {
            command_key: key("complete-legacy-no-ui", legacy_release_id.as_uuid()),
            build_request_id: legacy_fixture.build,
            release_id: legacy_release_id,
            version: ReleaseVersion::parse("legacy-v1").expect("release version"),
            release_agent_id: ReleaseAgentId::new(),
            artifacts: vec![ReleaseArtifactInput {
                id: ReleaseArtifactId::new(),
                path: ArtifactPath::parse("bin/reviewer").expect("artifact path"),
                kind: ArtifactKind::Executable,
                mode: 0o555,
                content_hash: ContentHash::digest(b"legacy"),
                size_bytes: 6,
                media_type: String::from("application/octet-stream"),
                storage_key: Uuid::new_v4(),
            }],
        })
        .await
        .expect("legacy no-UI build remains supported");
    let legacy_ui_rows: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM release_ui_source_snapshots WHERE release_id = $1",
    )
    .bind(legacy_release_id.as_uuid())
    .fetch_one(&admin_pool)
    .await
    .expect("legacy UI row count");
    assert_eq!(legacy_ui_rows, 0);
}
