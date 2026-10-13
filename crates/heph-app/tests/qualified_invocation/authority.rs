use capability_domain::{
    RuntimeCredential, RuntimeCredentialGeneration, RuntimeInvocation, RuntimeSessionIdentity,
    RuntimeSessionStatus,
};
use heph_run::{RunAuthorityManager, RunState};
use runtime_authority::{RuntimeSessionRepository, StoredRuntimeSession};
use sqlx::PgPool;
use volume_trait::RunVolumeStore;

use super::fixture::Fixture;

async fn exact_issue_and_retry(pool: &PgPool, count: usize) {
    let f = Box::pin(Fixture::provisioning(pool, count)).await;
    let run = f.run().await;
    assert_eq!(run.state, RunState::Provisioning);
    let session_id = capability_domain::RuntimeSessionId::from_uuid(run.id.as_uuid());
    assert!(f.repository.find(session_id).await.unwrap().is_none());
    assert!(f.handoff_files().is_empty());
    let prior = f.birth.rows().await;
    let bootstrap = f.issue().await;
    assert_eq!(bootstrap.session_id(), run.id.as_uuid());
    assert_eq!(
        bootstrap.generation(),
        RuntimeCredentialGeneration::INITIAL.get()
    );
    assert!(bootstrap.runtime_git_credential().is_none());
    let session =
        assert_issued_identity_and_lease_ceiling(&f, &run, pool, session_id, count, &bootstrap)
            .await;
    let rows = f.rows().await;
    assert_eq!(rows["runtime"]["sessions"].as_array().unwrap().len(), 1);
    assert_eq!(rows["runtime"]["snapshots"].as_array().unwrap().len(), 1);
    assert_eq!(rows["runtime"]["bindings"].as_array().unwrap().len(), 2);
    assert_eq!(rows["runtime"]["git"], serde_json::json!([]));
    assert_eq!(rows["runtime"]["credentials"], serde_json::json!([]));
    assert_eq!(f.birth.rows().await, prior);
    let handoff = f.handoff_files();
    assert_eq!(handoff.len(), 1);
    assert_exact_pending_replay(&f, &run, &bootstrap, &session, &rows, &handoff).await;
    f.birth.physical.assert_unchanged();
    f.birth.birth.assert_historical_migrations(pool).await;
}

async fn assert_issued_identity_and_lease_ceiling(
    f: &Fixture,
    run: &heph_run::Run,
    pool: &PgPool,
    session_id: capability_domain::RuntimeSessionId,
    count: usize,
    bootstrap: &heph_runtime::RuntimeAuthorityBootstrap,
) -> StoredRuntimeSession {
    let session = f.session().await;
    assert_eq!(session.id, session_id);
    assert_eq!(session.status, RuntimeSessionStatus::PendingHandoff);
    assert_eq!(session.generation, RuntimeCredentialGeneration::INITIAL);
    assert_eq!(session.issued_at.unix_timestamp_nanos().rem_euclid(1000), 0);
    assert_eq!(
        session.expires_at.unix_timestamp_nanos().rem_euclid(1000),
        0
    );
    assert_eq!(
        session.expires_at - session.issued_at,
        time::Duration::hours(1)
    );
    let leases = f.volumes.leases_for_run(run.id).await.unwrap();
    assert_eq!(leases.len(), count);
    assert!(
        leases
            .iter()
            .all(|lease| session.expires_at <= lease.lease().expires_at)
    );
    let snapshot = f
        .repository
        .resolve_snapshot(run, authz_postgres::AUTHORIZATION_MODEL_VERSION)
        .await
        .unwrap();
    let identity = RuntimeSessionIdentity::new(
        session_id,
        snapshot.principal(),
        RuntimeInvocation::Run(run.id),
        &snapshot,
        session.issued_at,
        session.expires_at,
    )
    .unwrap();
    assert_eq!(session.snapshot_id, snapshot.id());
    assert_eq!(session.identity_hash, identity.normalized_hash());
    let stored_hash: Vec<u8> =
        sqlx::query_scalar("SELECT credential_hash FROM runtime_authority_sessions WHERE id=$1")
            .bind(run.id.as_uuid())
            .fetch_one(pool)
            .await
            .unwrap();
    let expected_hash = RuntimeCredential::from_secret(*bootstrap.credential())
        .storage_hash(session.id, session.generation);
    assert!(
        stored_hash.as_slice() == expected_hash.as_bytes().as_slice(),
        "persisted bearer hash equality"
    );
    session
}

async fn assert_exact_pending_replay(
    f: &Fixture,
    run: &heph_run::Run,
    bootstrap: &heph_runtime::RuntimeAuthorityBootstrap,
    session: &StoredRuntimeSession,
    rows: &serde_json::Value,
    handoff: &std::collections::BTreeMap<String, String>,
) {
    // Recreated App manager, different trusted TTL: only exact pending times and
    // the original encrypted credential may be replayed. No bearer is printed.
    let retry = f
        .manager_ttl(std::time::Duration::from_secs(7))
        .prepare(run)
        .await
        .unwrap()
        .bootstrap
        .unwrap();
    assert!(
        retry.credential() == bootstrap.credential(),
        "pending bearer equality"
    );
    assert_eq!(retry.session_id(), bootstrap.session_id());
    assert_eq!(retry.generation(), bootstrap.generation());
    assert!(retry.runtime_git_credential().is_none());
    assert_eq!(f.session().await, *session);
    assert_eq!(f.rows().await, *rows);
    assert_eq!(f.handoff_files(), *handoff);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private119 PostgreSQL/native Local prerequisites"]
async fn app_zero_selection_issuance_normalizes_hash_and_replays_exact_pending(pool: PgPool) {
    exact_issue_and_retry(&pool, 0).await;
}
#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private119 PostgreSQL/native Local prerequisites"]
async fn app_two_selection_issuance_binds_all_held_leases_and_replays_exact_pending(pool: PgPool) {
    exact_issue_and_retry(&pool, 2).await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private119 PostgreSQL/native Local prerequisites"]
async fn app_session_interval_is_clamped_to_real_complete_held_lease_expiry(pool: PgPool) {
    let f = Box::pin(Fixture::leasing(&pool, 2)).await;
    f.acquire_for(time::Duration::seconds(90)).await;
    let run = f.run().await;
    let leases = f.volumes.leases_for_run(run.id).await.unwrap();
    assert_eq!(leases.len(), 2);
    f.issue().await;
    let session = f.session().await;
    let earliest = leases
        .iter()
        .map(|held| held.lease().expires_at)
        .min()
        .unwrap();
    assert_eq!(session.expires_at, earliest);
    assert!(session.expires_at - session.issued_at <= time::Duration::seconds(90));
    let before = f.rows().await;
    let handoff = f.handoff_files();
    f.issue().await;
    assert_eq!(f.session().await, session);
    assert_eq!(f.rows().await, before);
    assert_eq!(f.handoff_files(), handoff);
    f.birth.physical.assert_unchanged();
}
