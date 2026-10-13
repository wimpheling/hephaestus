//! Controlled closed comparison data; no `PostgreSQL` producer provenance.

use capability_domain::{
    AuthorizationSnapshot, AuthorizationSnapshotId, RuntimeCredentialGeneration,
    RuntimeCredentialHash, RuntimeInvocation, RuntimeSessionId, RuntimeSessionIdentity,
    RuntimeSessionStatus, WorkloadKind, WorkloadPrincipal,
};
use runtime_authority::{ExpiredRuntimeHandoff, LiveRunSnapshotPins, StoredRuntimeSession};
use runtime_git_authority::{
    ExpiredRuntimeGitCeiling, ExpiredRuntimeGitHandoff, ExpiredRuntimeGitSessionMetadata,
    StoredRuntimeGitCredential,
};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

pub(super) fn generic(change: u8) -> ExpiredRuntimeHandoff {
    let run = Uuid::from_u128(if change == 8 { 11 } else { 1 });
    let pins = LiveRunSnapshotPins::new(
        run,
        Uuid::from_u128(2),
        Uuid::from_u128(3),
        Uuid::from_u128(4),
        Uuid::from_u128(5),
        Uuid::from_u128(6),
    )
    .unwrap();
    let principal = WorkloadPrincipal::new(
        WorkloadKind::AgentInstance,
        pins.instance(),
        pins.revision(),
    );
    let snapshot = AuthorizationSnapshot::new(
        AuthorizationSnapshotId::from_uuid(Uuid::from_u128(if change == 3 { 9 } else { 7 })),
        principal,
        if change == 2 {
            "changed-model"
        } else {
            "controlled-model"
        },
        vec![],
    )
    .unwrap();
    let issued = OffsetDateTime::UNIX_EPOCH + Duration::seconds(i64::from(change == 4));
    let expires = issued + Duration::seconds(60);
    let id = RuntimeSessionId::from_uuid(Uuid::from_u128(if change == 6 { 10 } else { 8 }));
    let identity = RuntimeSessionIdentity::new(
        id,
        principal,
        RuntimeInvocation::Run(run.into()),
        &snapshot,
        issued,
        expires,
    )
    .unwrap();
    let session = StoredRuntimeSession {
        id,
        snapshot_id: snapshot.id(),
        identity_hash: identity.normalized_hash(),
        generation: RuntimeCredentialGeneration::new(if change == 7 { 2 } else { 1 }).unwrap(),
        status: RuntimeSessionStatus::Expired,
        issued_at: issued,
        expires_at: expires,
        acknowledged_at: if change == 1 {
            Some(issued + Duration::seconds(1))
        } else {
            None
        },
        revoked_at: None,
    };
    ExpiredRuntimeHandoff::new(
        session,
        snapshot,
        identity,
        pins,
        None,
        RuntimeCredentialHash::from_bytes(if change == 5 { [12; 32] } else { [11; 32] }),
        expires,
    )
    .unwrap()
}

pub(super) fn git(original: &ExpiredRuntimeHandoff, present: bool) -> ExpiredRuntimeGitHandoff {
    let session = original.session();
    let ceiling = ExpiredRuntimeGitCeiling::new(
        original.pins().run(),
        ExpiredRuntimeGitSessionMetadata {
            session: session.id,
            snapshot: session.snapshot_id,
            identity_hash: session.identity_hash,
            generation: session.generation,
            status: session.status,
            issued_at: session.issued_at,
            expires_at: session.expires_at,
            acknowledged_at: session.acknowledged_at,
            revoked_at: session.revoked_at,
        },
        original.snapshot().clone(),
        original.identity().clone(),
        original.credential_hash(),
        original.observed_at() + Duration::seconds(1),
    )
    .unwrap();
    if present {
        let credential = StoredRuntimeGitCredential {
            runtime_session_id: session.id,
            authorization_snapshot_id: session.snapshot_id,
            binding_id: Uuid::from_u128(20),
            repository_id: git_capability_domain::RepositoryId::new(Uuid::from_u128(21)),
            scope_hash: git_capability_domain::GitCapabilityHash::from_bytes([22; 32]),
            generation: session.generation,
            expires_at: session.expires_at,
        };
        ExpiredRuntimeGitHandoff::present(ceiling, credential, [23; 32]).unwrap()
    } else {
        ExpiredRuntimeGitHandoff::absent(ceiling)
    }
}
