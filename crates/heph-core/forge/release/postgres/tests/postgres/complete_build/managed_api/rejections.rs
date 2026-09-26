use super::*;

/// Runs each invalid UI capture case and checks that publication stays atomic.
pub(super) async fn verify(admin_pool: &PgPool, service: &ReleaseService, success_gateway: &str) {
    missing_artifact(admin_pool, service).await;
    wrong_media_type(admin_pool, service).await;
    oversized(admin_pool, service).await;
    foreign_agent(admin_pool, service).await;
    wrong_build_hash(admin_pool, service, success_gateway).await;
    wrong_gateway_hash(admin_pool, service, success_gateway).await;
    wrong_ui_hash(admin_pool, service).await;
    no_build(admin_pool, service).await;
}

async fn missing_artifact(admin_pool: &PgPool, service: &ReleaseService) {
    let missing_artifact = seed(admin_pool).await;
    attach_ui_capture(
        admin_pool,
        missing_artifact.build,
        static_ui_manifest("text/html"),
        None,
        None,
    )
    .await;
    assert_rejected_ui_build(
        service,
        admin_pool,
        missing_artifact.build,
        ReleaseId::new(),
        vec![release_test_artifact(
            "other.html",
            ArtifactKind::File,
            "text/html",
            10,
        )],
    )
    .await;
}

async fn wrong_media_type(admin_pool: &PgPool, service: &ReleaseService) {
    let wrong_media_type = seed(admin_pool).await;
    attach_ui_capture(
        admin_pool,
        wrong_media_type.build,
        static_ui_manifest("text/html"),
        None,
        None,
    )
    .await;
    assert_rejected_ui_build(
        service,
        admin_pool,
        wrong_media_type.build,
        ReleaseId::new(),
        vec![release_test_artifact(
            "dist/index.html",
            ArtifactKind::File,
            "text/plain",
            10,
        )],
    )
    .await;
}

async fn oversized(admin_pool: &PgPool, service: &ReleaseService) {
    let oversized = seed(admin_pool).await;
    attach_ui_capture(
        admin_pool,
        oversized.build,
        static_ui_manifest("text/html"),
        None,
        None,
    )
    .await;
    assert_rejected_ui_build(
        service,
        admin_pool,
        oversized.build,
        ReleaseId::new(),
        vec![release_test_artifact(
            "dist/index.html",
            ArtifactKind::File,
            "text/html",
            16 * 1024 * 1024 + 1,
        )],
    )
    .await;
}

async fn foreign_agent(admin_pool: &PgPool, service: &ReleaseService) {
    let foreign_agent = seed(admin_pool).await;
    let foreign_gateway = managed_gateway_manifest("unexported-agent");
    attach_ui_capture(
        admin_pool,
        foreign_agent.build,
        managed_api_manifest(),
        Some(foreign_gateway.as_bytes()),
        None,
    )
    .await;
    assert_rejected_ui_build(
        service,
        admin_pool,
        foreign_agent.build,
        ReleaseId::new(),
        vec![release_test_artifact(
            "bin/reviewer",
            ArtifactKind::Executable,
            "application/octet-stream",
            20,
        )],
    )
    .await;
}

async fn wrong_build_hash(admin_pool: &PgPool, service: &ReleaseService, success_gateway: &str) {
    let wrong_build_hash = seed(admin_pool).await;
    attach_ui_capture(
        admin_pool,
        wrong_build_hash.build,
        managed_api_manifest(),
        Some(success_gateway.as_bytes()),
        Some([7; 32]),
    )
    .await;
    assert_rejected_ui_build(
        service,
        admin_pool,
        wrong_build_hash.build,
        ReleaseId::new(),
        vec![release_test_artifact(
            "bin/reviewer",
            ArtifactKind::Executable,
            "application/octet-stream",
            20,
        )],
    )
    .await;
}

async fn wrong_gateway_hash(admin_pool: &PgPool, service: &ReleaseService, success_gateway: &str) {
    let wrong_gateway_hash = seed(admin_pool).await;
    attach_ui_capture_with_hashes(
        admin_pool,
        wrong_gateway_hash.build,
        managed_api_manifest(),
        Some(success_gateway.as_bytes()),
        None,
        None,
        Some([8; 32]),
    )
    .await;
    assert_rejected_ui_build(
        service,
        admin_pool,
        wrong_gateway_hash.build,
        ReleaseId::new(),
        vec![release_test_artifact(
            "bin/reviewer",
            ArtifactKind::Executable,
            "application/octet-stream",
            20,
        )],
    )
    .await;
}

async fn wrong_ui_hash(admin_pool: &PgPool, service: &ReleaseService) {
    let wrong_ui_hash = seed(admin_pool).await;
    attach_ui_capture_with_hashes(
        admin_pool,
        wrong_ui_hash.build,
        static_ui_manifest("text/html"),
        None,
        None,
        Some([8; 32]),
        None,
    )
    .await;
    assert_rejected_ui_build(
        service,
        admin_pool,
        wrong_ui_hash.build,
        ReleaseId::new(),
        vec![release_test_artifact(
            "dist/index.html",
            ArtifactKind::File,
            "text/html",
            10,
        )],
    )
    .await;
}

async fn no_build(admin_pool: &PgPool, service: &ReleaseService) {
    let no_build = seed(admin_pool).await;
    attach_ui_capture(
        admin_pool,
        no_build.build,
        static_ui_manifest("text/html"),
        None,
        None,
    )
    .await;
    sqlx::query(
        "UPDATE agent_config_revisions
            SET config = config - 'build'
          WHERE repository_id = (
                    SELECT repository_id FROM build_requests WHERE id = $1
                )
            AND commit_sha = repeat('a', 40)",
    )
    .bind(no_build.build.as_uuid())
    .execute(admin_pool)
    .await
    .expect("remove legacy build declaration");
    assert_rejected_ui_build(
        service,
        admin_pool,
        no_build.build,
        ReleaseId::new(),
        vec![release_test_artifact(
            "dist/index.html",
            ArtifactKind::File,
            "text/html",
            10,
        )],
    )
    .await;
}
