use std::time::Duration;

use authz_postgres::{AUTHORIZATION_MODEL_VERSION, begin_actor_transaction};
use hephaestus_app::recipe_instance_import::to_import_command;
use recipe_application::{
    DeploymentExecutionPreparation, DeploymentExecutionPreparationRepository,
};
use release_domain::{AgentInstanceId, AgentInstanceRevisionId, ReleaseCommandKey};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::{
    fixtures,
    instance_fixture::{self, Harness},
};

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private113 migrations"]
async fn first_preparation_reservation_refuses_matching_independent_birth_with_different_command(
    pool: PgPool,
) {
    let h = Harness::new(&pool, false).await;
    let before = instance_fixture::state(&pool).await;
    let prepared = h.prospective_slotless_preparation();
    let mut reserved = begin_actor_transaction(&h.app, &h.actor).await.unwrap();
    reserve(&mut reserved, &prepared).await;
    let mut foreign = to_import_command(&prepared.instances()[0]);
    foreign.command_key =
        ReleaseCommandKey::derive("foreign-instance", &[Uuid::new_v4().as_bytes()]);
    let denied = h
        .release
        .import_agent_with_volumes(&h.actor, foreign)
        .await
        .unwrap_err();
    match &denied {
        release_postgres::ReleaseServiceError::Database(error) => assert_eq!(
            error
                .as_database_error()
                .and_then(sqlx::error::DatabaseError::code)
                .as_deref(),
            Some("55P03"),
            "{denied:?}"
        ),
        _ => panic!("held instance reservation must be the actual denial: {denied:?}"),
    }
    assert_eq!(before, instance_fixture::state(&pool).await);
    write(&mut reserved, &prepared).await;
    reserved.commit().await.unwrap();
    assert_eq!(
        h.preparation
            .inspect_execution_preparation(&h.actor, h.intent.id())
            .await
            .unwrap(),
        Some(prepared)
    );
    h.importer()
        .import_prepared_instance(&h.actor, h.command, h.intent.id(), &fixtures::key("sqlite"))
        .await
        .unwrap();
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private113 migrations"]
async fn foreign_instance_cannot_borrow_reserved_command_after_first_preparation_commits(
    pool: PgPool,
) {
    let h = Harness::new(&pool, false).await;
    let prepared = h.prospective_slotless_preparation();
    let mut reserved = begin_actor_transaction(&h.app, &h.actor).await.unwrap();
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *reserved)
        .await
        .unwrap();
    reserve(&mut reserved, &prepared).await;
    let mut foreign = to_import_command(&prepared.instances()[0]);
    foreign.instance_id = AgentInstanceId::from_uuid(Uuid::new_v4());
    foreign.revision_id = AgentInstanceRevisionId::from_uuid(Uuid::new_v4());
    let id = foreign.instance_id;
    let service = h.release.clone();
    let actor = h.actor.clone();
    let pending =
        tokio::spawn(async move { service.import_agent_with_volumes(&actor, foreign).await });
    wait_for_command(&pool, pid).await;
    write(&mut reserved, &prepared).await;
    reserved.commit().await.unwrap();
    assert!(
        pending.await.unwrap().is_err(),
        "freshly reserved command rejects foreign birth atomically"
    );
    let rows: (i64,i64) = sqlx::query_as("SELECT (SELECT count(*) FROM agent_instances WHERE id=$1),(SELECT count(*) FROM release_command_inbox WHERE command_key=$2)")
        .bind(id.as_uuid()).bind(prepared.instances()[0].command_key().as_bytes().as_slice()).fetch_one(&pool).await.unwrap();
    assert_eq!(rows, (0, 0));
    assert!(h.active().await.is_none());
    h.importer()
        .import_prepared_instance(&h.actor, h.command, h.intent.id(), &fixtures::key("sqlite"))
        .await
        .unwrap();
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private113 migrations"]
async fn project_held_caller_fails_closed_at_instance_reservation_without_lock_cycle(pool: PgPool) {
    let h = Harness::new(&pool, false).await;
    let prepared = h.prospective_slotless_preparation();
    let child = &prepared.instances()[0];
    let before = instance_fixture::state(&pool).await;
    let mut caller = begin_actor_transaction(&h.app, &h.actor).await.unwrap();
    sqlx::query("SELECT id FROM projects WHERE id=$1 FOR NO KEY UPDATE")
        .bind(h.fixture.consuming_project)
        .fetch_one(&mut *caller)
        .await
        .unwrap();
    let mut reserved = begin_actor_transaction(&h.app, &h.actor).await.unwrap();
    // Reservation must succeed without taking the Project already held above.
    tokio::time::timeout(Duration::from_secs(10), reserve(&mut reserved, &prepared))
        .await
        .unwrap();
    let result = tokio::time::timeout(Duration::from_secs(10), sqlx::query("INSERT INTO agent_instances(id,project_id,family_id,name,state,created_by) SELECT $1,$2,family_id,'reserved-foreign','active',$3 FROM release_agents WHERE id=$4")
        .bind(child.instance_id().as_uuid()).bind(child.project_id().as_uuid()).bind(h.actor.user_id.as_uuid()).bind(child.source().pin().release_agent_id.as_uuid()).execute(&mut *caller)).await.unwrap().unwrap_err();
    assert_eq!(
        result
            .as_database_error()
            .and_then(sqlx::error::DatabaseError::code)
            .as_deref(),
        Some("55P03"),
        "{result:?}"
    );
    caller.rollback().await.unwrap();
    let mut raw_writer = begin_actor_transaction(&h.app, &h.actor).await.unwrap();
    sqlx::query("SELECT id FROM projects WHERE id=$1 FOR NO KEY UPDATE")
        .bind(h.fixture.consuming_project)
        .fetch_one(&mut *raw_writer)
        .await
        .unwrap();
    let denied = tokio::time::timeout(
        Duration::from_secs(10),
        sqlx::query("SELECT * FROM write_recipe_execution_preparation($1,$2,$3,$4,$5)")
            .bind(prepared.deployment_id().as_uuid())
            .bind(prepared.command().id().as_uuid())
            .bind(prepared.canonical_bytes().unwrap())
            .bind(children(&prepared))
            .bind(AUTHORIZATION_MODEL_VERSION)
            .fetch_one(&mut *raw_writer),
    )
    .await
    .unwrap()
    .unwrap_err();
    assert_eq!(
        denied
            .as_database_error()
            .and_then(sqlx::error::DatabaseError::code)
            .as_deref(),
        Some("55P03"),
        "{denied:?}"
    );
    raw_writer.rollback().await.unwrap();
    reserved.rollback().await.unwrap();
    assert_eq!(before, instance_fixture::state(&pool).await);
    assert!(
        h.preparation
            .inspect_execution_preparation(&h.actor, h.intent.id())
            .await
            .unwrap()
            .is_none()
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private113 migrations"]
async fn trusted_preparer_waits_on_exact107_command_mutex_before_taking_project_lock(pool: PgPool) {
    let h = Harness::new(&pool, false).await;
    let prepared = h.prospective_slotless_preparation();
    let mut reserved = begin_actor_transaction(&h.app, &h.actor).await.unwrap();
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *reserved)
        .await
        .unwrap();
    reserve(&mut reserved, &prepared).await;
    let repository = h.preparation.clone();
    let actor = h.actor.clone();
    let command = h.command;
    let deployment = h.intent.id();
    let pending = tokio::spawn(async move {
        repository
            .prepare_execution(&actor, command, deployment)
            .await
    });
    wait_for_command(&pool, pid).await;
    let mut probe = pool.begin().await.unwrap();
    sqlx::query("SELECT id FROM projects WHERE id=$1 FOR NO KEY UPDATE NOWAIT")
        .bind(h.fixture.consuming_project)
        .fetch_one(&mut *probe)
        .await
        .expect("blocked trusted preparer has not acquired Project");
    probe.rollback().await.unwrap();
    write(&mut reserved, &prepared).await;
    reserved.commit().await.unwrap();
    let replay = pending.await.unwrap().unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.preparation, prepared);
    assert!(h.active().await.is_none());
}

async fn reserve(tx: &mut Transaction<'_, Postgres>, preparation: &DeploymentExecutionPreparation) {
    sqlx::query("SELECT reserve_recipe_execution_preparation($1,$2,$3,$4)")
        .bind(preparation.deployment_id().as_uuid())
        .bind(preparation.command().id().as_uuid())
        .bind(preparation.canonical_bytes().unwrap())
        .bind(children(preparation))
        .execute(&mut **tx)
        .await
        .unwrap();
}

async fn write(tx: &mut Transaction<'_, Postgres>, preparation: &DeploymentExecutionPreparation) {
    sqlx::query("SELECT * FROM write_recipe_execution_preparation($1,$2,$3,$4,$5)")
        .bind(preparation.deployment_id().as_uuid())
        .bind(preparation.command().id().as_uuid())
        .bind(preparation.canonical_bytes().unwrap())
        .bind(children(preparation))
        .bind(AUTHORIZATION_MODEL_VERSION)
        .fetch_one(&mut **tx)
        .await
        .unwrap();
}

fn children(preparation: &DeploymentExecutionPreparation) -> Vec<Vec<u8>> {
    preparation
        .instances()
        .iter()
        .map(|child| serde_json::to_vec(child).unwrap())
        .collect()
}

async fn wait_for_command(pool: &PgPool, blocker: i32) {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let waiting: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity activity WHERE activity.datname=current_database() AND pg_blocking_pids(activity.pid) @> ARRAY[$1::integer] AND activity.query LIKE 'SELECT pg_advisory_xact_lock%')")
                .bind(blocker).fetch_one(pool).await.unwrap();
            if waiting { break; }
            tokio::task::yield_now().await;
        }
    }).await.expect("real107 command replay must wait on the actual reserved command mutex");
}
