use super::*;

struct CollisionState {
    collision_artifact: ReleaseArtifactInput,
    original_artifact: (Uuid, Uuid, String, String, i64, Uuid),
    post_write_failure: Fixture,
    post_write_release: ReleaseId,
    post_write_command: ReleaseCommandKey,
    colliding_artifact: ReleaseArtifactInput,
    family_count_before: i64,
}

/// Seeds the colliding artifact and prepares the post-write failure fixture.
async fn prepare(admin_pool: &PgPool, service: &ReleaseService) -> CollisionState {
    // Force the failure after release/family resolution and the release row
    // insert: a colliding artifact primary key must roll back that whole
    // publication transaction while preserving the original artifact.
    let collision_fixture = seed(admin_pool).await;
    let collision_release = ReleaseId::new();
    let collision_artifact =
        release_test_artifact("dist/index.html", ArtifactKind::File, "text/html", 10);
    service
        .complete_build(CompleteBuild {
            command_key: key("seed-artifact-collision", collision_release.as_uuid()),
            build_request_id: collision_fixture.build,
            release_id: collision_release,
            version: ReleaseVersion::parse("collision-v1").expect("release version"),
            release_agent_id: ReleaseAgentId::new(),
            artifacts: vec![collision_artifact.clone()],
        })
        .await
        .expect("seed colliding artifact");
    let original_artifact: (Uuid, Uuid, String, String, i64, Uuid) = sqlx::query_as(
        "SELECT id, release_id, path, media_type, size_bytes, storage_key
         FROM release_artifacts WHERE id = $1",
    )
    .bind(collision_artifact.id.as_uuid())
    .fetch_one(admin_pool)
    .await
    .expect("original colliding artifact");

    let post_write_failure = seed(admin_pool).await;
    attach_ui_capture(
        admin_pool,
        post_write_failure.build,
        static_ui_manifest("text/html"),
        None,
        None,
    )
    .await;
    let post_write_release = ReleaseId::new();
    let post_write_command = key(
        "post-write-artifact-collision",
        post_write_release.as_uuid(),
    );
    let mut colliding_artifact =
        release_test_artifact("dist/index.html", ArtifactKind::File, "text/html", 10);
    colliding_artifact.id = collision_artifact.id;
    let family_count_before: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM agent_families
         WHERE repository_id = (SELECT repository_id FROM build_requests WHERE id = $1)",
    )
    .bind(post_write_failure.build.as_uuid())
    .fetch_one(admin_pool)
    .await
    .expect("candidate family count before collision");
    CollisionState {
        collision_artifact,
        original_artifact,
        post_write_failure,
        post_write_release,
        post_write_command,
        colliding_artifact,
        family_count_before,
    }
}

async fn assert_collision_error(
    service: &ReleaseService,
    post_write_failure: &Fixture,
    post_write_release: ReleaseId,
    post_write_command: ReleaseCommandKey,
    colliding_artifact: ReleaseArtifactInput,
) {
    let collision_error = service
        .complete_build(CompleteBuild {
            command_key: post_write_command,
            build_request_id: post_write_failure.build,
            release_id: post_write_release,
            version: ReleaseVersion::parse("post-write-collision-v1").expect("release version"),
            release_agent_id: ReleaseAgentId::new(),
            artifacts: vec![colliding_artifact],
        })
        .await
        .expect_err("artifact primary-key collision must fail");
    match collision_error {
        release_postgres::ReleaseServiceError::Database(error) => {
            assert_eq!(
                error
                    .as_database_error()
                    .and_then(sqlx::error::DatabaseError::code),
                Some("23505".into())
            );
        }
        other => panic!("expected artifact collision database failure, got {other:?}"),
    }
}
async fn assert_rollback_counts(
    admin_pool: &PgPool,
    post_write_failure: &Fixture,
    post_write_release: ReleaseId,
    post_write_command: &ReleaseCommandKey,
    family_count_before: i64,
) {
    let rolled_back_counts: (i64, i64, i64, i64, i64, i64, i64, i64, i64, i64) = sqlx::query_as(
        "SELECT
                 (SELECT count(*) FROM releases WHERE id = $1),
                 (SELECT count(*) FROM release_artifacts WHERE release_id = $1),
                 (SELECT count(*) FROM release_agents WHERE release_id = $1),
                 (SELECT count(*) FROM release_ui_source_snapshots WHERE release_id = $1),
                 (SELECT count(*) FROM release_ui_descriptors WHERE release_id = $1),
                 (SELECT count(*) FROM release_ui_static_files WHERE release_id = $1),
                 (SELECT count(*) FROM release_ui_managed_services WHERE release_id = $1),
                 (SELECT count(*) FROM release_ui_api_bindings WHERE release_id = $1),
                 (SELECT count(*) FROM release_command_inbox WHERE command_key = $2),
                 (SELECT count(*) FROM agent_families
                    WHERE repository_id = (
                        SELECT repository_id FROM build_requests WHERE id = $3
                    ))",
    )
    .bind(post_write_release.as_uuid())
    .bind(post_write_command.as_bytes().as_slice())
    .bind(post_write_failure.build.as_uuid())
    .fetch_one(admin_pool)
    .await
    .expect("rolled-back publication counts");
    assert_eq!(
        rolled_back_counts,
        (0, 0, 0, 0, 0, 0, 0, 0, 0, family_count_before)
    );
    let candidate_state: String =
        sqlx::query_scalar("SELECT state FROM build_requests WHERE id = $1")
            .bind(post_write_failure.build.as_uuid())
            .fetch_one(admin_pool)
            .await
            .expect("candidate build state after collision");
    assert_eq!(candidate_state, "importing");
}
async fn assert_preserved_artifact(
    admin_pool: &PgPool,
    collision_artifact: &ReleaseArtifactInput,
    original_artifact: &(Uuid, Uuid, String, String, i64, Uuid),
) {
    let preserved_artifact: (Uuid, Uuid, String, String, i64, Uuid) = sqlx::query_as(
        "SELECT id, release_id, path, media_type, size_bytes, storage_key
         FROM release_artifacts WHERE id = $1",
    )
    .bind(collision_artifact.id.as_uuid())
    .fetch_one(admin_pool)
    .await
    .expect("preserved original artifact");
    assert_eq!(&preserved_artifact, original_artifact);
}
/// Proves the collision rolls back all release publication side effects.
async fn assert_rollback(admin_pool: &PgPool, service: &ReleaseService, state: CollisionState) {
    let CollisionState {
        collision_artifact,
        original_artifact,
        post_write_failure,
        post_write_release,
        post_write_command,
        colliding_artifact,
        family_count_before,
    } = state;
    assert_collision_error(
        service,
        &post_write_failure,
        post_write_release,
        post_write_command,
        colliding_artifact,
    )
    .await;
    assert_rollback_counts(
        admin_pool,
        &post_write_failure,
        post_write_release,
        &post_write_command,
        family_count_before,
    )
    .await;
    assert_preserved_artifact(admin_pool, &collision_artifact, &original_artifact).await;
}

/// Runs the failure phase after preparing the collision fixtures.
pub(super) async fn verify(admin_pool: &PgPool, service: &ReleaseService) {
    let state = prepare(admin_pool, service).await;
    assert_rollback(admin_pool, service, state).await;
}
