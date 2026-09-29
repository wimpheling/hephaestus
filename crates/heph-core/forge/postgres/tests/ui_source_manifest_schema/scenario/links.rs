use super::Context;
use crate::support::{OTHER_COMMIT, VALID_COMMIT, assert_sqlstate, commit};

pub async fn run(context: &Context) {
    let fixture = context.fixture;
    let linked: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM build_request_ui_source_manifests
         WHERE build_request_id = $1",
    )
    .bind(fixture.public_build)
    .fetch_one(&context.worker)
    .await
    .expect("valid source link");
    assert_eq!(linked, 0, "link is added after the shape checks");

    sqlx::query(
        "INSERT INTO build_request_ui_source_manifests
         (build_request_id, repository_id, source_commit,
          source_manifest_revision_id)
         VALUES ($1, $2, $3, $4)",
    )
    .bind(fixture.public_build)
    .bind(fixture.public_repository)
    .bind(commit(VALID_COMMIT))
    .bind(fixture.public_revision)
    .execute(&context.worker)
    .await
    .expect("link valid source revision");

    let invalid_link = sqlx::query(
        "INSERT INTO build_request_ui_source_manifests
         (build_request_id, repository_id, source_commit,
          source_manifest_revision_id)
         VALUES ($1, $2, $3, $4)",
    )
    .bind(fixture.invalid_build)
    .bind(fixture.public_repository)
    .bind(commit("c"))
    .bind(fixture.oversized_revision)
    .execute(&context.worker)
    .await;
    assert!(invalid_link.is_err(), "invalid source must not be linkable");
    assert_sqlstate(invalid_link, "23503");

    let cross_repository_link = sqlx::query(
        "INSERT INTO build_request_ui_source_manifests
         (build_request_id, repository_id, source_commit,
          source_manifest_revision_id)
         VALUES ($1, $2, $3, $4)",
    )
    .bind(fixture.private_build)
    .bind(fixture.public_repository)
    .bind(commit(VALID_COMMIT))
    .bind(fixture.public_revision)
    .execute(&context.worker)
    .await;
    assert!(
        cross_repository_link.is_err(),
        "cross-repository link must fail"
    );
    assert_sqlstate(cross_repository_link, "23503");

    let cross_commit_link = sqlx::query(
        "INSERT INTO build_request_ui_source_manifests
         (build_request_id, repository_id, source_commit,
          source_manifest_revision_id)
         VALUES ($1, $2, $3, $4)",
    )
    .bind(fixture.other_commit_build)
    .bind(fixture.public_repository)
    .bind(commit(OTHER_COMMIT))
    .bind(fixture.public_revision)
    .execute(&context.worker)
    .await;
    assert!(cross_commit_link.is_err(), "cross-commit link must fail");
    assert_sqlstate(cross_commit_link, "23503");

    sqlx::query(
        "INSERT INTO build_request_ui_source_manifests
         (build_request_id, repository_id, source_commit,
          source_manifest_revision_id)
         VALUES ($1, $2, $3, $4)",
    )
    .bind(fixture.second_public_build)
    .bind(fixture.public_repository)
    .bind(commit(VALID_COMMIT))
    .bind(fixture.public_revision)
    .execute(&context.worker)
    .await
    .expect("second matching build may share source snapshot");

    assert_immutable(context, fixture).await;
}

async fn assert_immutable(context: &Context, fixture: crate::support::Fixture) {
    let immutable_update = sqlx::query(
        "UPDATE ui_source_manifest_revisions
         SET diagnostics = '[{\"code\":\"changed\"}]'::jsonb
         WHERE id = $1",
    )
    .bind(fixture.public_revision)
    .execute(&context.admin)
    .await;
    assert!(
        immutable_update.is_err(),
        "source revisions must be immutable"
    );

    let immutable_delete =
        sqlx::query("DELETE FROM build_request_ui_source_manifests WHERE build_request_id = $1")
            .bind(fixture.public_build)
            .execute(&context.admin)
            .await;
    assert!(immutable_delete.is_err(), "source links must be immutable");
}
