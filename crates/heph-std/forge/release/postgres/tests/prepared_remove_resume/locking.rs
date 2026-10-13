use super::fixture::Fixture;
use authz_postgres::begin_actor_transaction;
use recipe_postgres::PostgresDeploymentRepository;
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::time::Duration;

#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private118 database"]
async fn owning_resume_waits_on_exact_original_command_before_parent_locks(pool: PgPool) {
    let f = Fixture::new(&pool).await;
    let app = PgPoolOptions::new()
        .max_connections(1)
        .after_connect(|connection, _| {
            Box::pin(async move {
                sqlx::query("SET ROLE hephaestus_app")
                    .execute(connection)
                    .await?;
                Ok(())
            })
        })
        .connect_with(pool.connect_options().as_ref().clone())
        .await
        .unwrap();
    let waiting: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&app)
        .await
        .unwrap();
    let mut blocker = pool.begin().await.unwrap();
    let blocked_by: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *blocker)
        .await
        .unwrap();
    let key = i64::from_be_bytes(
        *f.closure
            .original
            .base
            .input
            .command_key()
            .as_bytes()
            .first_chunk::<8>()
            .unwrap(),
    );
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(key)
        .execute(&mut *blocker)
        .await
        .unwrap();
    let before = f.snapshot().await;
    // Raw entry never waits, even if a caller could already hold late children.
    let mut raw = begin_actor_transaction(&f.closure.original.base.app, &f.manager)
        .await
        .unwrap();
    let error = sqlx::query("SELECT reserve_remove_resume118($1)")
        .bind(f.remove.deployment_id.as_uuid())
        .execute(&mut *raw)
        .await
        .unwrap_err();
    assert_eq!(
        error
            .as_database_error()
            .and_then(sqlx::error::DatabaseError::code)
            .as_deref(),
        Some("55P03")
    );
    raw.rollback().await.unwrap();
    assert_eq!(f.snapshot().await, before);
    let repository =
        PostgresDeploymentRepository::new(app.clone()).with_remove_resume(f.config.clone());
    let manager = f.manager.clone();
    let command = f.remove;
    let task = tokio::spawn(async move { repository.admit_remove_resume(&manager, command).await });
    tokio::time::timeout(Duration::from_secs(5),async {loop {
        let waiting_here:bool=sqlx::query_scalar("SELECT $2=ANY(pg_blocking_pids($1)) AND EXISTS(SELECT 1 FROM pg_stat_activity WHERE pid=$1 AND wait_event_type='Lock' AND wait_event='advisory')")
            .bind(waiting).bind(blocked_by).fetch_one(&pool).await.unwrap();
        if waiting_here {break;}tokio::task::yield_now().await;
    }}).await.unwrap();
    let mut probe = pool.begin().await.unwrap();
    sqlx::query("SELECT id FROM projects WHERE id=$1 FOR UPDATE NOWAIT")
        .bind(f.closure.original.base.intent.project_id().as_uuid())
        .execute(&mut *probe)
        .await
        .unwrap();
    sqlx::query("SELECT id FROM agent_instances WHERE id=$1 FOR UPDATE NOWAIT")
        .bind(f.closure.original.base.input.instance_id().as_uuid())
        .execute(&mut *probe)
        .await
        .unwrap();
    probe.rollback().await.unwrap();
    assert_eq!(f.snapshot().await, before);
    blocker.commit().await.unwrap();
    task.await.unwrap().unwrap();
    app.close().await;
}
#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private118 database"]
async fn late_participant_lock_fails_without_admission_or_events(pool: PgPool) {
    let f = Fixture::new(&pool).await;
    let mut blocker = pool.begin().await.unwrap();
    sqlx::query("SELECT id FROM agent_instances WHERE id=$1 FOR UPDATE")
        .bind(f.closure.original.base.input.instance_id().as_uuid())
        .execute(&mut *blocker)
        .await
        .unwrap();
    let before = f.snapshot().await;
    let mut raw = begin_actor_transaction(&f.closure.original.base.app, &f.manager)
        .await
        .unwrap();
    let error = sqlx::query("SELECT lock_remove_resume118($1)")
        .bind(f.remove.deployment_id.as_uuid())
        .execute(&mut *raw)
        .await
        .unwrap_err();
    assert_eq!(
        error
            .as_database_error()
            .and_then(sqlx::error::DatabaseError::code)
            .as_deref(),
        Some("55P03")
    );
    raw.rollback().await.unwrap();
    assert!(
        f.repository()
            .admit_remove_resume(&f.manager, f.remove)
            .await
            .is_err()
    );
    assert_eq!(f.snapshot().await, before);
    blocker.rollback().await.unwrap();
    f.repository()
        .admit_remove_resume(&f.manager, f.remove)
        .await
        .unwrap();
}
#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private118 database"]
async fn raw_admission_cannot_replace_selected_original_claim_or_mutate_lineage(pool: PgPool) {
    let f = Fixture::new(&pool).await;
    f.repository()
        .admit_remove_resume(&f.manager, f.remove)
        .await
        .unwrap();
    let before = f.snapshot().await;
    let mut tx = begin_actor_transaction(&f.closure.original.base.app, &f.manager)
        .await
        .unwrap();
    let allowed: bool = sqlx::query_scalar(
        "SELECT has_table_privilege(current_user,'recipe_remove_resume_admissions','UPDATE')",
    )
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    assert!(!allowed);
    let error = sqlx::query(
        "UPDATE recipe_remove_resume_admissions SET context_json='{}'::jsonb WHERE command_id=$1",
    )
    .bind(f.remove.command.id().as_uuid())
    .execute(&mut *tx)
    .await
    .unwrap_err();
    assert_eq!(
        error
            .as_database_error()
            .and_then(sqlx::error::DatabaseError::code)
            .as_deref(),
        Some("42501")
    );
    tx.rollback().await.unwrap();
    assert_eq!(f.snapshot().await, before);
    // The owner reaches the unconditional immutability trigger; actors have
    // no UPDATE privilege and cannot be used as evidence that it ran.
    let mut owner = pool.begin().await.unwrap();
    let error = sqlx::query(
        "UPDATE recipe_remove_resume_admissions SET context_json='{}'::jsonb WHERE command_id=$1",
    )
    .bind(f.remove.command.id().as_uuid())
    .execute(&mut *owner)
    .await
    .unwrap_err();
    assert_eq!(
        error
            .as_database_error()
            .and_then(sqlx::error::DatabaseError::code)
            .as_deref(),
        Some("23000")
    );
    owner.rollback().await.unwrap();
    assert_eq!(f.snapshot().await, before);
    let mut tx = begin_actor_transaction(&f.closure.original.base.app, &f.manager)
        .await
        .unwrap();
    let error=sqlx::query("INSERT INTO recipe_remove_resume_admissions SELECT command_id,deployment_id,project_id,expected_version,actor_id,request_id,event_id,
        intent_hash,preparation_hash,vm_provider_namespace,vm_host_id,platform_json,jsonb_set(context_json,'{claims,0,claim,generation}','999'::jsonb) FROM recipe_remove_resume_admissions WHERE command_id=$1")
        .bind(f.remove.command.id().as_uuid()).execute(&mut *tx).await.unwrap_err();
    assert_eq!(
        error
            .as_database_error()
            .and_then(sqlx::error::DatabaseError::code)
            .as_deref(),
        Some("23000")
    );
    tx.rollback().await.unwrap();
    assert_eq!(f.snapshot().await, before);
}
