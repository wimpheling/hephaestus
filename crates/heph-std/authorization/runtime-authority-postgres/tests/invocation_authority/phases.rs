use super::harness::{Harness, pg_time};
use capability_domain::{RuntimeCredentialGeneration, RuntimeSessionStatus};
use run_domain::RunState;
use run_orchestrator::RunRepository;
use runtime_authority::{LiveRunSnapshotResolver, RuntimeSessionRepository};
use sqlx::PgPool;

#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private119 PostgreSQL/native Local prerequisites"]
async fn starting_running_require_acknowledged_complete_attached_set_and_monitor_existing_ceiling(
    pool: PgPool,
) {
    let h = Box::pin(Harness::new(&pool, 2)).await;
    let stored = h.issue().await;
    let before = h.rows().await;
    assert!(
        h.birth
            .repository()
            .transition(h.birth.start.run_id, RunState::Starting, None, None)
            .await
            .is_err()
    );
    assert_eq!(h.rows().await, before);
    let active = h
        .repository
        .acknowledge(stored.id, RuntimeCredentialGeneration::INITIAL, pg_time())
        .await
        .unwrap();
    assert_eq!(active.status, RuntimeSessionStatus::Active);
    let before = h.rows().await;
    assert!(
        h.birth
            .repository()
            .transition(h.birth.start.run_id, RunState::Starting, None, None)
            .await
            .is_err()
    );
    assert_eq!(
        h.rows().await,
        before,
        "acknowledgement alone cannot invent attachment"
    );
    let acknowledged = h.rows().await;
    assert_eq!(
        h.repository
            .acknowledge(stored.id, RuntimeCredentialGeneration::INITIAL, pg_time())
            .await
            .unwrap(),
        active
    );
    assert_eq!(
        h.rows().await,
        acknowledged,
        "Provisioning acknowledgement replay never renews"
    );
    h.attach_metadata().await;
    for state in [RunState::Starting, RunState::Running] {
        h.birth
            .repository()
            .transition(h.birth.start.run_id, state, None, None)
            .await
            .unwrap();
        let before = h.rows().await;
        assert_eq!(
            h.repository
                .resolve_live_snapshot(
                    &h.ceiling(&active),
                    h.snapshot.authorization_model_version(),
                    pg_time()
                )
                .await
                .unwrap(),
            h.snapshot
        );
        assert!(
            h.repository.create(h.session()).await.is_err(),
            "late phase cannot issue a new ceiling"
        );
        assert_eq!(h.rows().await, before);
    }
    let before = h.rows().await;
    assert!(
        h.repository
            .acknowledge(stored.id, RuntimeCredentialGeneration::INITIAL, pg_time())
            .await
            .is_err()
    );
    assert_eq!(
        h.rows().await,
        before,
        "late phase cannot renew or acknowledge a ceiling"
    );
    h.birth.physical.assert_unchanged();
}
#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private119 PostgreSQL/native Local prerequisites"]
async fn expired_pending_ceiling_cannot_be_redelivered_reissued_or_renewed(pool: PgPool) {
    let h = Box::pin(Harness::new(&pool, 0)).await;
    let stored = h.issue().await;
    assert_eq!(h.repository.expire(stored.expires_at).await.unwrap(), 1);
    let before = h.rows().await;
    let expired = h.repository.find(stored.id).await.unwrap().unwrap();
    assert_eq!(expired.status, RuntimeSessionStatus::Expired);
    assert!(h.repository.create(h.session()).await.is_err());
    assert!(
        h.repository
            .acknowledge(stored.id, RuntimeCredentialGeneration::INITIAL, pg_time())
            .await
            .is_err()
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
    assert_eq!(h.rows().await, before);
}
#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private119 PostgreSQL/native Local prerequisites"]
async fn admitted_remove_preserves_immutable_producer_but_blocks_existing_ceiling(pool: PgPool) {
    let h = Box::pin(Harness::new(&pool, 0)).await;
    let stored = h.issue().await;
    let manager = crate::support::identity(h.birth.birth.fixture.outsider);
    sqlx::query("INSERT INTO project_maintainers(project_id,user_id) VALUES($1,$2)")
        .bind(h.birth.birth.fixture.consuming_project)
        .bind(manager.user_id.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    let snapshot = h
        .birth
        .birth
        .recipe
        .inspect(&manager, h.birth.birth.intent.id())
        .await
        .unwrap();
    h.birth
        .birth
        .recipe
        .admit_remove(
            &manager,
            crate::fixtures::remove(&manager, h.birth.birth.intent.id(), snapshot.version),
        )
        .await
        .unwrap();
    let immutable: bool = sqlx::query_scalar("SELECT invocation119_immutable($1)")
        .bind(h.birth.start.run_id.as_uuid())
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(
        immutable,
        "recorded producer remains data after durable removal"
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
        .revoke(stored.id, pg_time(), "durable removal")
        .await
        .unwrap();
    assert_eq!(revoked.status, RuntimeSessionStatus::Revoked);
    assert_eq!(h.repository.find(stored.id).await.unwrap(), Some(revoked));
    super::harness::assert_ceiling_unchanged(&before, &h.rows().await);
}
