use super::fixture::Fixture;
use sqlx::PgPool;

#[sqlx::test(migrations = false)]
#[ignore = "requires actual private119 PostgreSQL"]
async fn raw_child_holder_and_contended_parent_return_held_without_partial_writes(pool: PgPool) {
    let h = Fixture::new(&pool, 0).await;
    h.consume().await.unwrap();
    let before = h.rows().await;
    let mut holder = pool.begin().await.unwrap();
    sqlx::query("SELECT 1 FROM projects WHERE id=$1 FOR NO KEY UPDATE")
        .bind(h.birth.fixture.consuming_project)
        .execute(&mut *holder)
        .await
        .unwrap();
    let mut raw = h.birth.worker.begin().await.unwrap();
    sqlx::query("SELECT 1 FROM runs WHERE id=$1 FOR UPDATE")
        .bind(h.start.run_id.as_uuid())
        .execute(&mut *raw)
        .await
        .unwrap();
    let error = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        sqlx::query("SELECT lock_qualified_invocation119($1,$2)")
            .bind(h.start.run_id.as_uuid())
            .bind(serde_json::to_vec(&h.config.canonical_json()).unwrap())
            .execute(&mut *raw),
    )
    .await
    .unwrap()
    .unwrap_err();
    assert_eq!(code(&error).as_deref(), Some("55P03"), "{error:?}");
    raw.rollback().await.unwrap();
    holder.rollback().await.unwrap();
    assert_eq!(h.rows().await, before);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires actual private119 PostgreSQL/native ext4"]
async fn all_late_instance_run_volume_grant_locks_are_nonblocking_and_preserve_backings(
    pool: PgPool,
) {
    let h = Fixture::new(&pool, 2).await;
    h.consume().await.unwrap();
    let volume = h.physical.files[0].0;
    for query in [
        sqlx::query("SELECT 1 FROM agent_instances WHERE id=$1 FOR UPDATE")
            .bind(h.start.instance_id.as_uuid()),
        sqlx::query("SELECT 1 FROM runs WHERE id=$1 FOR UPDATE").bind(h.start.run_id.as_uuid()),
        sqlx::query("SELECT 1 FROM agent_instance_state_volumes WHERE id=$1 FOR UPDATE")
            .bind(volume),
        sqlx::query(
            "SELECT 1 FROM agent_instance_volume_mount_grants WHERE volume_id=$1 FOR UPDATE",
        )
        .bind(volume),
    ] {
        let before = h.rows().await;
        let mut holder = pool.begin().await.unwrap();
        query.execute(&mut *holder).await.unwrap();
        let mut raw = h.birth.worker.begin().await.unwrap();
        let error = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            sqlx::query("SELECT lock_qualified_invocation119($1,$2)")
                .bind(h.start.run_id.as_uuid())
                .bind(serde_json::to_vec(&h.config.canonical_json()).unwrap())
                .execute(&mut *raw),
        )
        .await
        .unwrap()
        .unwrap_err();
        assert_eq!(code(&error).as_deref(), Some("55P03"), "{error:?}");
        raw.rollback().await.unwrap();
        holder.rollback().await.unwrap();
        assert_eq!(h.rows().await, before);
        h.physical.assert_unchanged();
    }
}

#[sqlx::test(migrations = false)]
#[ignore = "requires actual private119 PostgreSQL/native ext4"]
async fn late_member_lock_denies_whole_acquisition_without_first_member_fence_change(pool: PgPool) {
    use run_orchestrator::RunRepository;
    use volume_trait::RunVolumeMetadataRepository;
    let h = Fixture::new(&pool, 2).await;
    h.consume().await.unwrap();
    h.repository()
        .transition(
            h.start.run_id,
            run_domain::RunState::LeasingVolume,
            None,
            None,
        )
        .await
        .unwrap();
    let selected = h
        .volumes()
        .load_run_selections(h.start.run_id)
        .await
        .unwrap();
    let before = h.rows().await;
    let mut ids = h
        .physical
        .files
        .iter()
        .map(|file| file.0)
        .collect::<Vec<_>>();
    ids.sort_unstable();
    let mut holder = pool.begin().await.unwrap();
    sqlx::query("SELECT 1 FROM agent_instance_state_volumes WHERE id=$1 FOR UPDATE")
        .bind(ids[1])
        .execute(&mut *holder)
        .await
        .unwrap();
    let now = time::OffsetDateTime::now_utc();
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            h.volumes().acquire_run(
                &selected,
                h.scope.host_id(),
                now,
                now + time::Duration::minutes(5)
            )
        )
        .await
        .unwrap()
        .is_err()
    );
    holder.rollback().await.unwrap();
    assert_eq!(h.rows().await, before);
    h.physical.assert_unchanged();
}

pub fn code(error: &sqlx::Error) -> Option<String> {
    error
        .as_database_error()
        .and_then(sqlx::error::DatabaseError::code)
        .map(std::borrow::Cow::into_owned)
}
