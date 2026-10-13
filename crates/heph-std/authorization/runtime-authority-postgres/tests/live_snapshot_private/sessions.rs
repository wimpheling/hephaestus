use capability_domain::{
    AuthorizationSnapshot, RuntimeCredential, RuntimeCredentialGeneration, RuntimeInvocation,
    RuntimeSessionId, RuntimeSessionIdentity,
};
use runtime_authority::{
    LiveRunSessionCeiling, LiveRunSnapshotRequest, LiveRunSnapshotResolver, NewRuntimeSession,
    RuntimeSessionRepository, StoredRuntimeSession,
};
use runtime_authority_postgres::PgRuntimeSessionRepository;
use time::{Duration, OffsetDateTime};

use super::fixture::{self, Execution};

pub async fn snapshot(
    repository: &PgRuntimeSessionRepository,
    e: &Execution,
) -> AuthorizationSnapshot {
    repository
        .resolve_live_snapshot(
            &LiveRunSnapshotRequest::pre_issuance(fixture::pins(e)),
            "live-fixture/v1",
            OffsetDateTime::now_utc(),
        )
        .await
        .unwrap()
}

pub async fn issue(
    repository: &PgRuntimeSessionRepository,
    e: &Execution,
    snapshot: &AuthorizationSnapshot,
) -> StoredRuntimeSession {
    issue_at(
        repository,
        e,
        snapshot,
        OffsetDateTime::now_utc() - Duration::seconds(1),
        OffsetDateTime::now_utc() + Duration::minutes(10),
    )
    .await
}

pub async fn issue_at(
    repository: &PgRuntimeSessionRepository,
    e: &Execution,
    snapshot: &AuthorizationSnapshot,
    issued: OffsetDateTime,
    expires: OffsetDateTime,
) -> StoredRuntimeSession {
    let id = RuntimeSessionId::from_uuid(e.run);
    let generation = RuntimeCredentialGeneration::INITIAL;
    let identity = RuntimeSessionIdentity::new(
        id,
        snapshot.principal(),
        RuntimeInvocation::Run(e.run.into()),
        snapshot,
        issued,
        expires,
    )
    .unwrap();
    let credential = RuntimeCredential::from_secret([17; 32]);
    repository
        .create(NewRuntimeSession {
            snapshot,
            identity: &identity,
            generation,
            credential_hash: credential.storage_hash(id, generation),
            attachment_id: e.attachment,
        })
        .await
        .unwrap();
    drop(credential);
    // PostgreSQL timestamptz precision is authoritative for replay pins.
    repository.find(id).await.unwrap().unwrap()
}

pub fn request(
    e: &Execution,
    session: &StoredRuntimeSession,
    snapshot: &AuthorizationSnapshot,
) -> LiveRunSnapshotRequest {
    LiveRunSnapshotRequest::issued(
        fixture::pins(e),
        LiveRunSessionCeiling::new(session, snapshot.normalized_hash()).unwrap(),
    )
    .unwrap()
}

pub async fn allowed(
    repository: &PgRuntimeSessionRepository,
    request: &LiveRunSnapshotRequest,
    expected: &AuthorizationSnapshot,
) {
    let fresh = repository
        .resolve_live_snapshot(request, "live-fixture/v1", OffsetDateTime::now_utc())
        .await
        .unwrap();
    assert_eq!(&fresh, expected);
}
