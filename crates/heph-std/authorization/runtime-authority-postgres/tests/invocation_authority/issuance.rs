use super::harness::{Harness, pg_time};
use capability_domain::{RuntimeCredentialGeneration, RuntimeSessionIdentity};
use runtime_authority::{NewRuntimeSession, RuntimeSessionRepository};
use sqlx::PgPool;
use volume_trait::RunVolumeMetadataRepository;

#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private119 PostgreSQL/native Local prerequisites"]
async fn slotless_provisioning_issues_exact_microsecond_ceiling_and_replays_original(pool: PgPool) {
    let h = Box::pin(Harness::new(&pool, 0)).await;
    assert_eq!(
        h.repository.find(h.identity.id()).await.unwrap(),
        None,
        "genuine absent graph with current eligible original producer"
    );
    let first = h.issue().await;
    assert_eq!(first.issued_at, h.identity.issued_at());
    assert_eq!(first.expires_at, h.identity.expires_at());
    assert_eq!(first.identity_hash, h.identity.normalized_hash());
    let source = h
        .repository
        .recorded_live_source_context(&h.ceiling(&first))
        .await
        .unwrap();
    assert_eq!(source.actor_id(), h.birth.birth.actor.user_id.as_uuid());
    assert_eq!(source.pins(), h.pins());
    let before = h.rows().await;
    assert_eq!(
        h.repository.find(first.id).await.unwrap(),
        Some(first.clone())
    );
    assert_eq!(h.rows().await, before);
    assert!(h.repository.create(h.session()).await.is_err());
    assert_eq!(
        h.rows().await,
        before,
        "no second verifier/generation/ceiling"
    );
    h.birth.physical.assert_unchanged();
}
#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private119 PostgreSQL/native Local prerequisites"]
async fn exact_3600_maximum_shorter_lease_and_fractional_original_claims(pool: PgPool) {
    let mut h = Box::pin(Harness::new(&pool, 0)).await;
    let original = h.identity.clone();
    for excess in [
        time::Duration::microseconds(1),
        time::Duration::nanoseconds(1),
    ] {
        h.identity = RuntimeSessionIdentity::new(
            original.id(),
            original.principal(),
            original.invocation(),
            &h.snapshot,
            original.issued_at(),
            original.expires_at() + excess,
        )
        .unwrap();
        let before = h.rows().await;
        assert!(h.repository.create(h.session()).await.is_err());
        assert!(
            h.raw_writer(&h.birth.birth.worker, &h.identity)
                .await
                .is_err()
        );
        assert_eq!(h.rows().await, before);
    }
    h.identity = original;
    h.issue().await;
}
#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private119 PostgreSQL/native Local prerequisites"]
async fn session_expiry_cannot_exceed_current_held_lease(pool: PgPool) {
    let bound = Box::pin(Harness::new(&pool, 2)).await;
    let selections = bound
        .birth
        .volumes()
        .load_run_selections(bound.birth.start.run_id)
        .await
        .unwrap();
    let now = pg_time();
    bound
        .birth
        .volumes()
        .heartbeat_run(
            &selections,
            bound.birth.scope.host_id(),
            now,
            now + time::Duration::minutes(10),
        )
        .await
        .unwrap();
    let before = bound.rows().await;
    assert!(bound.repository.create(bound.session()).await.is_err());
    assert_eq!(
        bound.rows().await,
        before,
        "expiry cannot exceed any current held lease"
    );
    let capped = RuntimeSessionIdentity::new(
        bound.identity.id(),
        bound.identity.principal(),
        bound.identity.invocation(),
        &bound.snapshot,
        now,
        now + time::Duration::minutes(9),
    )
    .unwrap();
    let session = NewRuntimeSession {
        snapshot: &bound.snapshot,
        identity: &capped,
        generation: RuntimeCredentialGeneration::INITIAL,
        credential_hash: bound.credential,
        attachment_id: None,
    };
    bound.repository.create(session).await.unwrap();
}
#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private119 PostgreSQL/native Local prerequisites"]
async fn genuine_optional_capabilities_whole_ceiling_matches_core_and_preserves_parent_history(
    pool: PgPool,
) {
    let mut h = Box::pin(Harness::new(&pool, 2)).await;
    h.add_optional_bindings().await;
    let stored = h.issue().await;
    assert_eq!(stored.identity_hash, h.identity.normalized_hash());
    let sql_hash: Vec<u8> = sqlx::query_scalar("SELECT invocation119_snapshot_hash($1,$2)")
        .bind(h.birth.start.run_id.as_uuid())
        .bind(h.snapshot.authorization_model_version())
        .fetch_one(&h.birth.birth.worker)
        .await
        .unwrap();
    assert_eq!(sql_hash.as_slice(), h.snapshot.normalized_hash().as_bytes());
    let before = h.rows().await;
    assert_eq!(h.repository.find(stored.id).await.unwrap(), Some(stored));
    assert_eq!(h.rows().await, before);
    h.birth.birth.assert_historical_migrations(&pool).await;
    h.birth.physical.assert_unchanged();
}
