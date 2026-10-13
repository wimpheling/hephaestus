use capability_domain::{
    AuthorityHash, AuthorizationSnapshotId, RuntimeCredentialGeneration, RuntimeSessionId,
    RuntimeSessionStatus,
};
use runtime_authority::{
    LiveRunSessionCeiling, LiveRunSnapshotRequest, RuntimeSessionRepository, StoredRuntimeSession,
};
use sqlx::PgPool;
use time::{Duration, OffsetDateTime};

use super::{fixture, sessions};

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private profile loader"]
async fn issued_checks_require_actual_session_even_for_exact_pins(pool: PgPool) {
    let f = fixture::setup(&pool).await;
    let e = fixture::normal(&pool, &f, "owned_canonical").await;
    let repository = fixture::repository(&pool).await;
    let snapshot = sessions::snapshot(&repository, &e).await;
    fixture::advance(&pool, e.run, "starting").await;
    let now = OffsetDateTime::now_utc();
    let absent = StoredRuntimeSession {
        id: RuntimeSessionId::from_uuid(e.run),
        snapshot_id: AuthorizationSnapshotId::from_uuid(e.run),
        identity_hash: AuthorityHash::from_bytes([2; 32]),
        generation: RuntimeCredentialGeneration::INITIAL,
        status: RuntimeSessionStatus::PendingHandoff,
        issued_at: now - Duration::seconds(1),
        expires_at: now + Duration::minutes(10),
        acknowledged_at: None,
        revoked_at: None,
    };
    fixture::denied(&repository, &sessions::request(&e, &absent, &snapshot)).await;
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM runtime_authority_sessions WHERE run_id=$1")
            .bind(e.run)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(count, 0, "read-only resolution must not issue a session");
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private profile loader"]
async fn expired_and_permanently_revoked_sessions_deny_current_running_checks(pool: PgPool) {
    let f = fixture::setup(&pool).await;
    let expired = fixture::normal(&pool, &f, "owned_canonical").await;
    let repository = fixture::repository(&pool).await;
    let snapshot = sessions::snapshot(&repository, &expired).await;
    fixture::advance(&pool, expired.run, "provisioning").await;
    let now = OffsetDateTime::now_utc();
    let elapsed = sessions::issue_at(
        &repository,
        &expired,
        &snapshot,
        now - Duration::minutes(10),
        now - Duration::seconds(1),
    )
    .await;
    fixture::advance(&pool, expired.run, "running").await;
    fixture::denied(
        &repository,
        &sessions::request(&expired, &elapsed, &snapshot),
    )
    .await;
    let revoked = fixture::normal(&pool, &f, "owned_canonical").await;
    let snapshot = sessions::snapshot(&repository, &revoked).await;
    fixture::advance(&pool, revoked.run, "provisioning").await;
    let session = sessions::issue(&repository, &revoked, &snapshot).await;
    fixture::advance(&pool, revoked.run, "running").await;
    let request = sessions::request(&revoked, &session, &snapshot);
    sessions::allowed(&repository, &request, &snapshot).await;
    repository
        .revoke(
            session.id,
            OffsetDateTime::now_utc(),
            "live fixture revocation",
        )
        .await
        .unwrap();
    fixture::denied(&repository, &request).await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private profile loader"]
async fn issued_immutable_identity_generation_time_and_hash_cannot_substitute(pool: PgPool) {
    let f = fixture::setup(&pool).await;
    let e = fixture::normal(&pool, &f, "owned_canonical").await;
    let repository = fixture::repository(&pool).await;
    let snapshot = sessions::snapshot(&repository, &e).await;
    fixture::advance(&pool, e.run, "provisioning").await;
    let original = sessions::issue(&repository, &e, &snapshot).await;
    fixture::advance(&pool, e.run, "running").await;
    sessions::allowed(
        &repository,
        &sessions::request(&e, &original, &snapshot),
        &snapshot,
    )
    .await;
    for substitution in [
        Substitution::Identity,
        Substitution::Generation,
        Substitution::Issued,
        Substitution::Expiry,
    ] {
        let altered = substitution.apply(original.clone());
        fixture::denied(&repository, &sessions::request(&e, &altered, &snapshot)).await;
    }
    let mut foreign = original.clone();
    foreign.id = RuntimeSessionId::new();
    assert!(
        LiveRunSnapshotRequest::issued(
            fixture::pins(&e),
            LiveRunSessionCeiling::new(&foreign, snapshot.normalized_hash()).unwrap()
        )
        .is_err()
    );
    foreign = original.clone();
    foreign.snapshot_id = AuthorizationSnapshotId::new();
    assert!(
        LiveRunSnapshotRequest::issued(
            fixture::pins(&e),
            LiveRunSessionCeiling::new(&foreign, snapshot.normalized_hash()).unwrap()
        )
        .is_err()
    );
    let wrong_hash =
        LiveRunSessionCeiling::new(&original, AuthorityHash::from_bytes([99; 32])).unwrap();
    fixture::denied(
        &repository,
        &LiveRunSnapshotRequest::issued(fixture::pins(&e), wrong_hash).unwrap(),
    )
    .await;
}

#[derive(Clone, Copy)]
enum Substitution {
    Identity,
    Generation,
    Issued,
    Expiry,
}
impl Substitution {
    fn apply(self, mut session: StoredRuntimeSession) -> StoredRuntimeSession {
        match self {
            Self::Identity => session.identity_hash = AuthorityHash::from_bytes([91; 32]),
            Self::Generation => session.generation = RuntimeCredentialGeneration::new(2).unwrap(),
            Self::Issued => session.issued_at -= Duration::seconds(1),
            Self::Expiry => session.expires_at += Duration::seconds(1),
        }
        session
    }
}
