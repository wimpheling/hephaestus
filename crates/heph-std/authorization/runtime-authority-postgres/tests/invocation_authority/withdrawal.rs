use super::harness::{Harness, pg_time};
use run_orchestrator::RunRepository;
use runtime_authority::{LiveRunSnapshotResolver, RuntimeSessionRepository};
use sqlx::PgPool;

#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private119 PostgreSQL/native Local prerequisites"]
async fn source_withdrawal_denies_pending_redelivery_without_replacement_or_fence_release(
    pool: PgPool,
) {
    let h = Box::pin(Harness::new(&pool, 2)).await;
    let stored = h.issue().await;
    sqlx::query("DELETE FROM project_maintainers WHERE project_id=$1 AND user_id=$2")
        .bind(h.birth.birth.fixture.project)
        .bind(h.birth.birth.actor.user_id.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    let source_allowed: bool = sqlx::query_scalar(
        "SELECT check_permission('user',$1::text,'can_use','release_agent',$2::text)=1",
    )
    .bind(h.birth.birth.actor.user_id.as_uuid())
    .bind(h.birth.start.release_agent_id.as_uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        !source_allowed,
        "actual saved invoker source permission withdrawn"
    );
    let before = h.rows().await;
    assert!(
        h.repository.find(stored.id).await.is_err(),
        "existing withdrawn ceiling is never None"
    );
    assert!(
        h.repository
            .resolve_live_snapshot(
                &h.ceiling(&stored),
                h.snapshot.authorization_model_version(),
                pg_time()
            )
            .await
            .is_err()
    );
    assert!(h.repository.create(h.session()).await.is_err());
    assert_eq!(h.rows().await, before);
    let revoked = h
        .repository
        .revoke(stored.id, pg_time(), "source withdrawn")
        .await
        .unwrap();
    assert_eq!(
        revoked.status,
        capability_domain::RuntimeSessionStatus::Revoked
    );
    assert_eq!(h.repository.find(stored.id).await.unwrap(), Some(revoked));
    h.birth.physical.assert_unchanged();
}
#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private119 PostgreSQL/native Local prerequisites"]
async fn capability_issuer_withdrawal_denies_existing_whole_ceiling_not_subset(pool: PgPool) {
    let mut h = Box::pin(Harness::new(&pool, 2)).await;
    h.add_optional_bindings().await;
    let stored = h.issue().await;
    sqlx::query("DELETE FROM project_capability_granters WHERE project_id=$1 AND user_id=$2")
        .bind(h.birth.birth.fixture.consuming_project)
        .bind(h.birth.birth.actor.user_id.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    let rights:(bool,bool,bool)=sqlx::query_as("SELECT check_permission('user',$1::text,'can_execute','agent_instance',$2::text)=1,check_permission('user',$1::text,'can_use','release_agent',$3::text)=1,can_grant_agent_capability_operations($1,'project',$4,ARRAY['inspect'])")
  .bind(h.birth.birth.actor.user_id.as_uuid()).bind(h.birth.start.instance_id.as_uuid())
  .bind(h.birth.start.release_agent_id.as_uuid()).bind(h.birth.birth.fixture.consuming_project)
  .fetch_one(&pool).await.unwrap();
    assert_eq!(
        rights,
        (true, true, false),
        "issuer delegation alone is withdrawn"
    );
    let before = h.rows().await;
    assert!(h.repository.find(stored.id).await.is_err());
    assert!(
        h.repository
            .resolve_live_snapshot(
                &h.ceiling(&stored),
                h.snapshot.authorization_model_version(),
                pg_time()
            )
            .await
            .is_err()
    );
    assert!(h.repository.create(h.session()).await.is_err());
    assert_eq!(h.rows().await, before);
    let revoked = h
        .repository
        .revoke(stored.id, pg_time(), "issuer delegation withdrawn")
        .await
        .unwrap();
    assert_eq!(
        revoked.status,
        capability_domain::RuntimeSessionStatus::Revoked
    );
    assert_eq!(h.repository.find(stored.id).await.unwrap(), Some(revoked));
    super::harness::assert_ceiling_unchanged(&before, &h.rows().await);
    h.birth.physical.assert_unchanged();
}
#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private119 PostgreSQL/native Local prerequisites"]
async fn suspended_saved_invoker_cannot_issue_or_redeliver_existing_pending_session(pool: PgPool) {
    let h = Box::pin(Harness::new(&pool, 0)).await;
    let stored = h.issue().await;
    sqlx::query("UPDATE users SET status='suspended' WHERE id=$1")
        .bind(h.birth.birth.actor.user_id.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    let before = h.rows().await;
    assert!(h.repository.find(stored.id).await.is_err());
    assert!(h.repository.create(h.session()).await.is_err());
    assert_eq!(h.rows().await, before);
    assert_eq!(h.repository.expire(stored.expires_at).await.unwrap(), 1);
    let expired = h.repository.find(stored.id).await.unwrap().unwrap();
    assert_eq!(
        expired.status,
        capability_domain::RuntimeSessionStatus::Expired
    );
    super::harness::assert_ceiling_unchanged(&before, &h.rows().await);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private119 PostgreSQL/native Local prerequisites"]
async fn active_ceiling_and_broker_wire_deny_source_loss_but_cleanup_metadata_remains_readable(
    pool: PgPool,
) {
    let h = Box::pin(Harness::new(&pool, 2)).await;
    let pending = h.issue().await;
    let active = h
        .repository
        .acknowledge(
            pending.id,
            capability_domain::RuntimeCredentialGeneration::INITIAL,
            pg_time(),
        )
        .await
        .unwrap();
    h.attach_metadata().await;
    h.birth
        .repository()
        .transition(
            h.birth.start.run_id,
            run_domain::RunState::Starting,
            None,
            None,
        )
        .await
        .unwrap();
    let broker:(uuid::Uuid,uuid::Uuid,Option<uuid::Uuid>,i64)=sqlx::query_as("SELECT instance_id,instance_revision_id,attachment_id,issuance_generation FROM authenticate_runtime_authority($1)")
  .bind(h.credential.as_bytes().as_slice()).fetch_one(&h.birth.birth.app).await.unwrap();
    assert_eq!(
        broker,
        (
            h.birth.start.instance_id.as_uuid(),
            h.birth.start.instance_revision_id.as_uuid(),
            None,
            1
        )
    );
    sqlx::query("DELETE FROM project_maintainers WHERE project_id=$1 AND user_id=$2")
        .bind(h.birth.birth.fixture.project)
        .bind(h.birth.birth.actor.user_id.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    let before = h.rows().await;
    assert_eq!(
        h.repository.find(active.id).await.unwrap(),
        Some(active.clone()),
        "active record is cleanup data, never permission to issue"
    );
    assert!(
        h.repository
            .resolve_live_snapshot(
                &h.ceiling(&active),
                h.snapshot.authorization_model_version(),
                pg_time()
            )
            .await
            .is_err()
    );
    let broker: Option<uuid::Uuid> =
        sqlx::query_scalar("SELECT session_id FROM authenticate_runtime_authority($1)")
            .bind(h.credential.as_bytes().as_slice())
            .fetch_optional(&h.birth.birth.app)
            .await
            .unwrap();
    assert!(
        broker.is_none(),
        "broker cannot impersonate original user or attachment after source withdrawal"
    );
    assert!(h.repository.create(h.session()).await.is_err());
    assert_eq!(h.rows().await, before);
    h.birth.physical.assert_unchanged();
}
