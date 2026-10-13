//! Controlled closed comparison models, not `PostgreSQL` original qualification.

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
use time::OffsetDateTime;
use uuid::Uuid;

pub fn generic(
    id: RuntimeSessionId,
    generation: RuntimeCredentialGeneration,
    expires: OffsetDateTime,
    verifier: RuntimeCredentialHash,
) -> ExpiredRuntimeHandoff {
    let pins = LiveRunSnapshotPins::new(
        Uuid::from_u128(1),
        Uuid::from_u128(2),
        Uuid::from_u128(3),
        Uuid::from_u128(4),
        Uuid::from_u128(5),
        Uuid::from_u128(6),
    )
    .unwrap();
    let snapshot = AuthorizationSnapshot::new(
        AuthorizationSnapshotId::from_uuid(Uuid::from_u128(7)),
        WorkloadPrincipal::new(
            WorkloadKind::AgentInstance,
            pins.instance(),
            pins.revision(),
        ),
        "controlled-filesystem-model",
        vec![],
    )
    .unwrap();
    let issued = expires - time::Duration::minutes(1);
    let identity = RuntimeSessionIdentity::new(
        id,
        snapshot.principal(),
        RuntimeInvocation::Run(pins.run().into()),
        &snapshot,
        issued,
        expires,
    )
    .unwrap();
    let session = StoredRuntimeSession {
        id,
        snapshot_id: snapshot.id(),
        identity_hash: identity.normalized_hash(),
        generation,
        status: RuntimeSessionStatus::Expired,
        issued_at: issued,
        expires_at: expires,
        acknowledged_at: None,
        revoked_at: None,
    };
    ExpiredRuntimeHandoff::new(session, snapshot, identity, pins, None, verifier, expires).unwrap()
}

fn ceiling(
    id: RuntimeSessionId,
    generation: RuntimeCredentialGeneration,
    expires: OffsetDateTime,
) -> ExpiredRuntimeGitCeiling {
    let original = generic(
        id,
        generation,
        expires,
        RuntimeCredentialHash::from_bytes([19; 32]),
    );
    let session = original.session();
    ExpiredRuntimeGitCeiling::new(
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
        expires,
    )
    .unwrap()
}

pub fn git(
    id: RuntimeSessionId,
    generation: RuntimeCredentialGeneration,
    expires: OffsetDateTime,
    verifier: [u8; 32],
) -> ExpiredRuntimeGitHandoff {
    let ceiling = ceiling(id, generation, expires);
    let credential = StoredRuntimeGitCredential {
        runtime_session_id: id,
        authorization_snapshot_id: ceiling.snapshot().id(),
        binding_id: Uuid::from_u128(8),
        repository_id: git_capability_domain::RepositoryId::new(Uuid::from_u128(9)),
        scope_hash: git_capability_domain::GitCapabilityHash::from_bytes([17; 32]),
        generation,
        expires_at: expires,
    };
    ExpiredRuntimeGitHandoff::present(ceiling, credential, verifier).unwrap()
}

pub fn no_git(
    id: RuntimeSessionId,
    generation: RuntimeCredentialGeneration,
    expires: OffsetDateTime,
) -> ExpiredRuntimeGitHandoff {
    ExpiredRuntimeGitHandoff::absent(ceiling(id, generation, expires))
}
