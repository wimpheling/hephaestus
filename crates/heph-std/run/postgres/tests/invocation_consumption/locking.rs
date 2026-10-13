use sqlx::PgPool;

use super::{fixture::Fixture, integrity::code};

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and private117"]
async fn held_instance_and_run_fail_nowait_without_parent_child_deadlock(pool: PgPool) {
    let h = Fixture::new(&pool).await;
    for query in [
        sqlx::query("SELECT 1 FROM agent_instances WHERE id=$1 FOR UPDATE"),
        sqlx::query("SELECT 1 FROM runs WHERE instance_id=$1 FOR UPDATE"),
    ] {
        let before = h.rows().await;
        let mut holder = pool.begin().await.unwrap();
        query
            .bind(h.start.instance_id.as_uuid())
            .execute(&mut *holder)
            .await
            .unwrap();
        let mut worker = h.producer.activation.worker.begin().await.unwrap();
        let error = tokio::time::timeout(std::time::Duration::from_secs(5), h.write(&mut worker))
            .await
            .unwrap()
            .unwrap_err();
        assert_eq!(code(&error).as_deref(), Some("55P03"), "{error:?}");
        worker.rollback().await.unwrap();
        holder.rollback().await.unwrap();
        assert_eq!(h.rows().await, before);
    }
    h.consume().await.unwrap();
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL/native ext4 and private117"]
async fn held_global_volume_and_grant_fail_nowait_preserving_actual_backing(pool: PgPool) {
    let h = Fixture::bound(&pool).await;
    for query in [
        sqlx::query("SELECT 1 FROM agent_instance_state_volumes WHERE id=$1 FOR UPDATE"),
        sqlx::query(
            "SELECT 1 FROM agent_instance_volume_mount_grants WHERE volume_id=$1 FOR UPDATE",
        ),
    ] {
        let before = h.rows().await;
        let physical = h.producer.activation.physical.as_ref().unwrap();
        let mut holder = pool.begin().await.unwrap();
        query
            .bind(physical.volume)
            .execute(&mut *holder)
            .await
            .unwrap();
        let mut worker = h.producer.activation.worker.begin().await.unwrap();
        let error = tokio::time::timeout(std::time::Duration::from_secs(5), h.write(&mut worker))
            .await
            .unwrap()
            .unwrap_err();
        assert_eq!(code(&error).as_deref(), Some("55P03"), "{error:?}");
        worker.rollback().await.unwrap();
        holder.rollback().await.unwrap();
        assert_eq!(h.rows().await, before);
        physical.assert_unchanged();
    }
    assert!(!h.consume().await.unwrap().created);
    h.producer
        .activation
        .physical
        .as_ref()
        .unwrap()
        .assert_unchanged();
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and private117"]
async fn project_held_raw_caller_and_reserved_command_return_held_not_deadlock(pool: PgPool) {
    let h = Fixture::new(&pool).await;
    let before = h.rows().await;
    let mut owner = pool.begin().await.unwrap();
    let key: i64 = sqlx::query_scalar("SELECT recipe_import107_command_mutex(command_key) FROM agent_instance_execution_invocations WHERE run_id=$1")
        .bind(h.start.run_id.as_uuid()).fetch_one(&mut *owner).await.unwrap();
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(key)
        .execute(&mut *owner)
        .await
        .unwrap();
    let mut raw = h.producer.activation.worker.begin().await.unwrap();
    sqlx::query("SELECT 1 FROM projects WHERE id=$1 FOR NO KEY UPDATE")
        .bind(h.producer.activation.fixture.consuming_project)
        .execute(&mut *raw)
        .await
        .unwrap();
    let error = tokio::time::timeout(std::time::Duration::from_secs(5), h.write(&mut raw))
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(code(&error).as_deref(), Some("55P03"), "{error:?}");
    raw.rollback().await.unwrap();
    owner.rollback().await.unwrap();
    assert_eq!(h.rows().await, before);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and private117"]
async fn raw_child_holder_denies_contended_project_without_parent_child_cycle(pool: PgPool) {
    let h = Fixture::new(&pool).await;
    let before = h.rows().await;
    let mut parent = pool.begin().await.unwrap();
    sqlx::query("SELECT 1 FROM projects WHERE id=$1 FOR NO KEY UPDATE")
        .bind(h.producer.activation.fixture.consuming_project)
        .execute(&mut *parent)
        .await
        .unwrap();
    let mut raw = h.producer.activation.worker.begin().await.unwrap();
    // A direct worker already owns the child, the inverse of ordinary removal.
    // Its protected entry must fail rather than block on the held parent.
    sqlx::query("SELECT 1 FROM runs WHERE id=$1 FOR UPDATE")
        .bind(h.start.run_id.as_uuid())
        .execute(&mut *raw)
        .await
        .unwrap();
    let error = tokio::time::timeout(std::time::Duration::from_secs(5), h.write(&mut raw))
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(code(&error).as_deref(), Some("55P03"), "{error:?}");
    raw.rollback().await.unwrap();
    parent.rollback().await.unwrap();
    // Includes original inbox, immutable producer, both outboxes and receipts.
    assert_eq!(h.rows().await, before);
    assert!(!h.consume().await.unwrap().created);
}
