use serial_test::serial;
use std::env;
use tokio::sync::oneshot;
use uuid::Uuid;

use super::support::{
    admin_pool, assert_sqlstate, role_pool, seed_parent_rows, wait_until_blocked,
};

#[tokio::test]
#[serial]
// Keep the two-connection barrier sequence together so its lock ordering is
// auditable in the schema proof.
#[allow(clippy::too_many_lines)]
async fn generation_insert_barrier_rejects_concurrent_source_project_move() {
    let Some(database_url) = env::var("HEPHAESTUS_POSTGRES_TEST_URL").ok() else {
        eprintln!("skipping generation/source-parent barrier: test URL is unset");
        return;
    };
    let bootstrap = admin_pool(&database_url).await;
    sqlx::migrate!("../../../../../migrations")
        .run(&bootstrap)
        .await
        .expect("apply migrations through 0088");

    let generation_pool = role_pool(&database_url, "hephaestus_worker", Uuid::nil()).await;
    let parent_pool = role_pool(&database_url, "hephaestus_worker", Uuid::nil()).await;
    let fixture = seed_parent_rows(&generation_pool).await;
    let source_organization: Uuid =
        sqlx::query_scalar("SELECT organization_id FROM projects WHERE id = $1")
            .bind(fixture.project)
            .fetch_one(&generation_pool)
            .await
            .expect("read source organization");
    let moved_organization = Uuid::new_v4();
    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, 'Barrier destination')")
        .bind(moved_organization)
        .execute(&generation_pool)
        .await
        .expect("seed destination organization");

    let installation_id = Uuid::new_v4();
    let generation_id = Uuid::new_v4();
    let mut generation_transaction = generation_pool
        .begin()
        .await
        .expect("begin generation transaction");
    let generation_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *generation_transaction)
        .await
        .expect("read generation transaction PID");
    sqlx::query(
        "INSERT INTO ui_installations
         (id, organization_id, project_id, repository_id, scope, ui_key,
          lifecycle, current_generation_id, created_by)
         VALUES ($1, $2, NULL, NULL, 'global', 'schema-global-two',
                 'enabled', $3, $4)",
    )
    .bind(installation_id)
    .bind(source_organization)
    .bind(generation_id)
    .bind(fixture.actor)
    .execute(&mut *generation_transaction)
    .await
    .expect("insert barrier installation");
    sqlx::query(
        "INSERT INTO ui_installation_generations
         (id, installation_id, generation_no, release_id, ui_key, ui_scope)
         VALUES ($1, $2, 1, $3, 'schema-global-two', 'global')",
    )
    .bind(generation_id)
    .bind(installation_id)
    .bind(fixture.second_release)
    .execute(&mut *generation_transaction)
    .await
    .expect("insert deferred barrier generation");

    let mut parent_transaction = parent_pool.begin().await.expect("begin parent move");
    let parent_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *parent_transaction)
        .await
        .expect("read parent transaction PID");
    sqlx::query("UPDATE projects SET organization_id = $2 WHERE id = $1")
        .bind(fixture.project)
        .bind(moved_organization)
        .execute(&mut *parent_transaction)
        .await
        .expect("uncommitted source move passes before generation is visible");

    let (commit_started_sender, commit_started_receiver) = oneshot::channel();
    let generation_commit = tokio::spawn(async move {
        commit_started_sender
            .send(())
            .expect("generation commit receiver");
        generation_transaction.commit().await
    });
    commit_started_receiver
        .await
        .expect("generation commit started");
    wait_until_blocked(&bootstrap, generation_pid, parent_pid).await;
    parent_transaction
        .commit()
        .await
        .expect("source move commits before generation validation");
    let generation_result = generation_commit
        .await
        .expect("generation commit task join");
    assert_sqlstate(generation_result, "23000");

    let remaining_rows: (i64, i64) = sqlx::query_as(
        "SELECT
            (SELECT count(*) FROM ui_installations WHERE id = $1),
            (SELECT count(*) FROM ui_installation_generations WHERE id = $2)",
    )
    .bind(installation_id)
    .bind(generation_id)
    .fetch_one(&bootstrap)
    .await
    .expect("inspect rolled-back generation");
    assert_eq!(remaining_rows, (0, 0));
}
