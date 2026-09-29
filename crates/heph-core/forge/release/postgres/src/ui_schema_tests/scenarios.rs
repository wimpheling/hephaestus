use super::{
    constraints, fixtures, immutable, rls, rows,
    support::{admin_pool, assert_sqlstate, assert_state, wait_until_blocked},
};
use serial_test::serial;
use std::env;
use tokio::sync::oneshot;

#[tokio::test]
#[serial]
// Keep the end-to-end schema lifecycle assertions in their original order.
#[allow(clippy::too_many_lines)]
async fn release_ui_schema_enforces_identity_shape_lifecycle_and_rls() {
    let Some(url) = env::var("HEPHAESTUS_POSTGRES_TEST_URL").ok() else {
        eprintln!("SKIPPED release UI schema: HEPHAESTUS_POSTGRES_TEST_URL is unset");
        return;
    };
    let admin = admin_pool(&url).await;
    sqlx::migrate!("../../../../../migrations")
        .run(&admin)
        .await
        .expect("apply migrations through 0084");
    let version: i64 =
        sqlx::query_scalar("SELECT max(version) FROM _sqlx_migrations WHERE success")
            .fetch_one(&admin)
            .await
            .expect("read migration version");
    assert!(
        version >= 84,
        "release UI publication schema and catalog grants must be installed"
    );

    let fixture = fixtures::seed_fixture(&admin).await;
    rows::seed_valid_ui_rows(&admin, &fixture).await;
    rows::assert_valid_rows(&admin, &fixture).await;
    constraints::assert_shape_constraints(&admin, &fixture).await;
    immutable::assert_immutable_rows(&admin, &fixture).await;
    rls::assert_rls_isolation(&url, &fixture).await;

    sqlx::query(
        "UPDATE releases
         SET state = 'published', published_at = now(), publication_actor_id = $2
         WHERE id = $1 AND state = 'draft'",
    )
    .bind(fixture.release)
    .bind(fixture.owner)
    .execute(&admin)
    .await
    .expect("draft release publishes after UI rows are inserted");
    sqlx::query(
        "UPDATE releases
         SET state = 'published', published_at = now(), publication_actor_id = $2
         WHERE id = $1 AND state = 'draft'",
    )
    .bind(fixture.release_without_ui)
    .bind(fixture.owner)
    .execute(&admin)
    .await
    .expect("empty draft release publishes for source-snapshot guard");
    assert_state(&admin, fixture.release, "published").await;
    immutable::assert_insert_guard(&admin, &fixture, "published").await;

    sqlx::query(
        "UPDATE releases SET state = 'revoked', revoked_at = now()
         WHERE id = $1 AND state = 'published'",
    )
    .bind(fixture.release)
    .execute(&admin)
    .await
    .expect("published release revokes");
    sqlx::query(
        "UPDATE releases SET state = 'revoked', revoked_at = now()
         WHERE id = $1 AND state = 'published'",
    )
    .bind(fixture.release_without_ui)
    .execute(&admin)
    .await
    .expect("empty published release revokes");
    assert_state(&admin, fixture.release, "revoked").await;
    immutable::assert_insert_guard(&admin, &fixture, "revoked").await;
}
#[tokio::test]
#[serial]
async fn release_ui_draft_guard_serializes_publish_and_ui_insert() {
    let Some(url) = env::var("HEPHAESTUS_POSTGRES_TEST_URL").ok() else {
        eprintln!("SKIPPED release UI draft guard: HEPHAESTUS_POSTGRES_TEST_URL is unset");
        return;
    };
    let admin = admin_pool(&url).await;
    sqlx::migrate!("../../../../../migrations")
        .run(&admin)
        .await
        .expect("apply migrations through 0084");
    let fixture = fixtures::seed_fixture(&admin).await;
    rows::seed_valid_ui_rows(&admin, &fixture).await;

    let mut insert_first = admin.begin().await.expect("begin insert-first transaction");
    let insert_first_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *insert_first)
        .await
        .expect("read insert-first PID");
    sqlx::query(
        "INSERT INTO release_ui_source_snapshots
         (release_id, build_request_id, source_manifest_revision_id)
         VALUES ($1, $2, $3)",
    )
    .bind(fixture.release_without_ui)
    .bind(fixture.build_without_ui)
    .bind(fixture.source_revision_without_ui)
    .execute(&mut *insert_first)
    .await
    .expect("insert source snapshot while draft lock is held");
    sqlx::query(
        "INSERT INTO release_ui_descriptors
         (release_id, ui_key, scope, label, icon, presentation, route_base,
          entrypoint, ui_kit_version, cache, content_kind)
         VALUES ($1, 'race-insert', 'global', 'Race insert', 'app', 'iframe',
                 'race-insert', 'index.html', 1, 'no_store', 'static')",
    )
    .bind(fixture.release_without_ui)
    .execute(&mut *insert_first)
    .await
    .expect("insert UI descriptor while draft lock is held");

    let (publish_pid_tx, publish_pid_rx) = oneshot::channel();
    let publish_pool = admin.clone();
    let publish_release = fixture.release_without_ui;
    let publish_owner = fixture.owner;
    let publish_task = tokio::spawn(async move {
        let mut transaction = publish_pool.begin().await?;
        let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *transaction)
            .await?;
        publish_pid_tx.send(pid).expect("publish PID receiver");
        sqlx::query(
            "UPDATE releases
             SET state = 'published', published_at = now(), publication_actor_id = $2
             WHERE id = $1 AND state = 'draft'",
        )
        .bind(publish_release)
        .bind(publish_owner)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await
    });
    let publish_pid = publish_pid_rx.await.expect("publish PID");
    wait_until_blocked(&admin, publish_pid, insert_first_pid).await;
    insert_first
        .commit()
        .await
        .expect("commit insert before publish");
    publish_task
        .await
        .expect("publish task join")
        .expect("publish after insert commit");
    assert_state(&admin, fixture.release_without_ui, "published").await;
    let inserted_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM release_ui_descriptors WHERE release_id = $1")
            .bind(fixture.release_without_ui)
            .fetch_one(&admin)
            .await
            .expect("count inserted descriptor");
    assert_eq!(inserted_count, 1);

    let mut publish_first = admin
        .begin()
        .await
        .expect("begin publish-first transaction");
    sqlx::query(
        "UPDATE releases
         SET state = 'published', published_at = now(), publication_actor_id = $2
         WHERE id = $1 AND state = 'draft'",
    )
    .bind(fixture.release)
    .bind(fixture.owner)
    .execute(&mut *publish_first)
    .await
    .expect("hold published release row lock");
    let publish_first_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *publish_first)
        .await
        .expect("read publish-first PID");
    let (insert_pid_tx, insert_pid_rx) = oneshot::channel();
    let insert_pool = admin.clone();
    let insert_release = fixture.release;
    let insert_artifact = fixture.artifact;
    let insert_task = tokio::spawn(async move {
        let mut transaction = insert_pool.begin().await?;
        let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *transaction)
            .await?;
        insert_pid_tx.send(pid).expect("insert PID receiver");
        sqlx::query(
            "INSERT INTO release_ui_static_files
             (release_id, ui_key, route, artifact_id, artifact_kind, artifact_media_type)
             VALUES ($1, 'docs', 'after-publish.html', $2, 'file', 'text/html')",
        )
        .bind(insert_release)
        .bind(insert_artifact)
        .execute(&mut *transaction)
        .await
        .map(|_| ())
    });
    let insert_pid = insert_pid_rx.await.expect("insert PID");
    wait_until_blocked(&admin, insert_pid, publish_first_pid).await;
    publish_first
        .commit()
        .await
        .expect("commit publish before blocked insert resumes");
    let insert_result = insert_task.await.expect("insert task join");
    assert_sqlstate(insert_result, "23000");
    let static_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM release_ui_static_files WHERE release_id = $1")
            .bind(fixture.release)
            .fetch_one(&admin)
            .await
            .expect("count static rows after rejected insert");
    assert_eq!(static_count, 1);
}
