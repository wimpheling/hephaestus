use serial_test::serial;
use std::env;
use tokio::sync::oneshot;
use uuid::Uuid;

use super::support::{
    admin_pool, assert_sqlstate, role_pool, seed_draft_global_release, seed_global_installation,
    seed_parent_rows, wait_until_blocked,
};

#[tokio::test]
#[serial]
// Keep the two-connection source-release lock barrier beside the retained
// generation parent guard; the source-project barrier covers re-read failure.
#[allow(clippy::too_many_lines)]
async fn generation_barrier_locks_draft_source_release_before_mutation() {
    let Some(database_url) = env::var("HEPHAESTUS_POSTGRES_TEST_URL").ok() else {
        eprintln!("skipping source-release barrier: test URL is unset");
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
    let destination_organization = Uuid::new_v4();
    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, 'Release barrier destination')")
        .bind(destination_organization)
        .execute(&generation_pool)
        .await
        .expect("seed release destination organization");

    let draft_release = Uuid::new_v4();
    let draft_ui_key = "schema-release-barrier";
    seed_draft_global_release(
        &generation_pool,
        fixture.release,
        draft_release,
        draft_ui_key,
    )
    .await;
    let installation_id = Uuid::new_v4();
    let generation_id = Uuid::new_v4();
    let mut generation_transaction = generation_pool
        .begin()
        .await
        .expect("begin source-release generation transaction");
    let generation_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *generation_transaction)
        .await
        .expect("read source-release generation PID");
    sqlx::query(
        "INSERT INTO ui_installations
         (id, organization_id, project_id, repository_id, scope, ui_key,
          lifecycle, current_generation_id, created_by)
         VALUES ($1, $2, NULL, NULL, 'global', $3, 'enabled', $4, $5)",
    )
    .bind(installation_id)
    .bind(source_organization)
    .bind(draft_ui_key)
    .bind(generation_id)
    .bind(fixture.actor)
    .execute(&mut *generation_transaction)
    .await
    .expect("insert source-release barrier installation");
    sqlx::query(
        "INSERT INTO ui_installation_generations
         (id, installation_id, generation_no, release_id, ui_key, ui_scope)
         VALUES ($1, $2, 1, $3, $4, 'global')",
    )
    .bind(generation_id)
    .bind(installation_id)
    .bind(draft_release)
    .bind(draft_ui_key)
    .execute(&mut *generation_transaction)
    .await
    .expect("insert source-release barrier generation");

    let mut release_move_transaction = parent_pool
        .begin()
        .await
        .expect("begin draft source-release move");
    let release_move_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *release_move_transaction)
        .await
        .expect("read source-release move PID");
    sqlx::query("UPDATE releases SET version = $2 WHERE id = $1")
        .bind(draft_release)
        .bind(format!("ui-barrier-moved-{draft_release}"))
        .execute(&mut *release_move_transaction)
        .await
        .expect("draft source-release mutation is initially unreferenced");

    let (commit_started_sender, commit_started_receiver) = oneshot::channel();
    let generation_commit = tokio::spawn(async move {
        commit_started_sender
            .send(())
            .expect("source-release commit receiver");
        generation_transaction.commit().await
    });
    commit_started_receiver
        .await
        .expect("source-release generation commit started");
    wait_until_blocked(&bootstrap, generation_pid, release_move_pid).await;
    release_move_transaction
        .commit()
        .await
        .expect("draft source-release mutation commits before validation");
    let generation_result = generation_commit
        .await
        .expect("source-release generation commit task join");
    generation_result.expect("generation validation succeeds after source-release lock");

    let remaining_rows: (i64, i64) = sqlx::query_as(
        "SELECT
            (SELECT count(*) FROM ui_installations WHERE id = $1),
            (SELECT count(*) FROM ui_installation_generations WHERE id = $2)",
    )
    .bind(installation_id)
    .bind(generation_id)
    .fetch_one(&bootstrap)
    .await
    .expect("inspect source-release barrier generation");
    assert_eq!(remaining_rows, (1, 1));

    let generation_first_key = "schema-release-retained";
    let generation_first_release = Uuid::new_v4();
    seed_draft_global_release(
        &generation_pool,
        fixture.release,
        generation_first_release,
        generation_first_key,
    )
    .await;
    let generation_first_installation = Uuid::new_v4();
    let generation_first_generation = Uuid::new_v4();
    seed_global_installation(
        &generation_pool,
        source_organization,
        generation_first_installation,
        generation_first_generation,
        generation_first_release,
        generation_first_key,
    )
    .await;
    let rejected_parent_move =
        sqlx::query("UPDATE projects SET organization_id = $2 WHERE id = $1")
            .bind(fixture.project)
            .bind(destination_organization)
            .execute(&parent_pool)
            .await;
    assert_sqlstate(rejected_parent_move, "23000");
}
