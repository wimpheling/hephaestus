//! Controlled comparison cases, not worker, filesystem or native qualification.

use super::*;
use capability_domain::WorkloadPrincipal;
use git_capability_domain::{GitCapabilityHash, RepositoryId};

#[path = "tests/ports.rs"]
mod ports;

struct Fixture {
    run: Uuid,
    metadata: ExpiredRuntimeGitSessionMetadata,
    snapshot: AuthorizationSnapshot,
    identity: RuntimeSessionIdentity,
}
impl Fixture {
    fn new(run: Uuid) -> Self {
        let principal = WorkloadPrincipal::new(
            WorkloadKind::AgentInstance,
            Uuid::from_u128(2),
            Uuid::from_u128(3),
        );
        let snapshot = AuthorizationSnapshot::new(
            AuthorizationSnapshotId::from_uuid(Uuid::from_u128(7)),
            principal,
            "controlled-model",
            vec![],
        )
        .unwrap();
        let issued = OffsetDateTime::UNIX_EPOCH;
        let expires = issued + time::Duration::seconds(60);
        let identity = RuntimeSessionIdentity::new(
            RuntimeSessionId::from_uuid(Uuid::from_u128(8)),
            principal,
            RuntimeInvocation::Run(run.into()),
            &snapshot,
            issued,
            expires,
        )
        .unwrap();
        let metadata = ExpiredRuntimeGitSessionMetadata {
            session: identity.id(),
            snapshot: snapshot.id(),
            identity_hash: identity.normalized_hash(),
            generation: RuntimeCredentialGeneration::INITIAL,
            status: RuntimeSessionStatus::Expired,
            issued_at: issued,
            expires_at: expires,
            acknowledged_at: Some(issued + time::Duration::seconds(1)),
            revoked_at: None,
        };
        Self {
            run,
            metadata,
            snapshot,
            identity,
        }
    }
    fn ceiling(self) -> Result<ExpiredRuntimeGitCeiling, RuntimeGitAuthorityError> {
        let observed = self.metadata.expires_at;
        ExpiredRuntimeGitCeiling::new(
            self.run,
            self.metadata,
            self.snapshot,
            self.identity,
            RuntimeCredentialHash::from_bytes([0xa7; 32]),
            observed,
        )
    }
}
fn binding(ceiling: &ExpiredRuntimeGitCeiling) -> StoredRuntimeGitCredential {
    StoredRuntimeGitCredential {
        runtime_session_id: ceiling.session().session,
        authorization_snapshot_id: ceiling.session().snapshot,
        binding_id: Uuid::from_u128(10),
        repository_id: RepositoryId::new(Uuid::from_u128(11)),
        scope_hash: GitCapabilityHash::from_bytes([0x1c; 32]),
        generation: ceiling.session().generation,
        expires_at: ceiling.session().expires_at,
    }
}
fn present(run: Uuid) -> ExpiredRuntimeGitHandoff {
    let ceiling = Fixture::new(run).ceiling().unwrap();
    let credential = binding(&ceiling);
    ExpiredRuntimeGitHandoff::present(ceiling, credential, [0xab; 32]).unwrap()
}

#[test]
fn present_and_absent_models_preserve_one_exact_terminal_ceiling() {
    let original = present(Uuid::from_u128(1));
    let ceiling = original.ceiling();
    assert_eq!(ceiling.run(), Uuid::from_u128(1));
    assert_eq!(ceiling.snapshot().id(), ceiling.identity().snapshot_id());
    assert_eq!(
        ceiling.session().identity_hash,
        ceiling.identity().normalized_hash()
    );
    assert_eq!(
        ceiling.generic_credential_hash(),
        RuntimeCredentialHash::from_bytes([0xa7; 32])
    );
    assert_eq!(ceiling.observed_at(), ceiling.session().expires_at);
    assert!(!ceiling.identity().is_valid_at(ceiling.observed_at()));
    assert_eq!(original.credential(), Some(&binding(ceiling)));
    assert_eq!(original.credential_hash(), Some(&[0xab; 32]));
    assert!(!format!("{original:?}").contains("171, 171"));
    let absent = ExpiredRuntimeGitHandoff::absent(ceiling.clone());
    assert_eq!(absent.ceiling(), original.ceiling());
    assert!(absent.credential().is_none());
    assert!(absent.credential_hash().is_none());
    assert!(absent.matches_request(
        ceiling.run(),
        ceiling.session().session,
        ceiling.session().generation
    ));
    assert!(!absent.matches_request(
        ceiling.session().session.as_uuid(),
        ceiling.session().session,
        ceiling.session().generation
    ));
    assert!(!absent.matches_request(
        ceiling.run(),
        RuntimeSessionId::from_uuid(Uuid::from_u128(99)),
        ceiling.session().generation
    ));
    assert!(!absent.matches_request(
        ceiling.run(),
        ceiling.session().session,
        RuntimeCredentialGeneration::new(2).unwrap()
    ));
}

#[test]
fn non_expired_or_conflicting_generic_originals_deny() {
    for status in [
        RuntimeSessionStatus::Active,
        RuntimeSessionStatus::PendingHandoff,
        RuntimeSessionStatus::Revoked,
    ] {
        let mut f = Fixture::new(Uuid::from_u128(1));
        f.metadata.status = status;
        assert_eq!(
            f.ceiling().unwrap_err(),
            RuntimeGitAuthorityError::IdentityMismatch
        );
    }
    let mut f = Fixture::new(Uuid::from_u128(1));
    f.run = Uuid::from_u128(99);
    assert!(f.ceiling().is_err());
    let mut f = Fixture::new(Uuid::from_u128(1));
    f.metadata.revoked_at = Some(f.metadata.issued_at);
    assert!(f.ceiling().is_err());
    let mut f = Fixture::new(Uuid::from_u128(1));
    f.metadata.identity_hash = AuthorityHash::from_bytes([0; 32]);
    assert!(f.ceiling().is_err());
    let mut f = Fixture::new(Uuid::from_u128(1));
    f.snapshot = AuthorizationSnapshot::new(
        f.snapshot.id(),
        f.snapshot.principal(),
        "changed-ceiling",
        vec![],
    )
    .unwrap();
    assert!(f.ceiling().is_err());
}

#[test]
fn every_git_binding_link_substitution_denies() {
    let ceiling = Fixture::new(Uuid::from_u128(1)).ceiling().unwrap();
    let original = binding(&ceiling);
    let mut wrong = original.clone();
    wrong.runtime_session_id = RuntimeSessionId::from_uuid(Uuid::from_u128(99));
    assert!(ExpiredRuntimeGitHandoff::present(ceiling.clone(), wrong, [1; 32]).is_err());
    let mut wrong = original.clone();
    wrong.authorization_snapshot_id = AuthorizationSnapshotId::from_uuid(Uuid::from_u128(99));
    assert!(ExpiredRuntimeGitHandoff::present(ceiling.clone(), wrong, [1; 32]).is_err());
    let mut wrong = original.clone();
    wrong.generation = RuntimeCredentialGeneration::new(2).unwrap();
    assert!(ExpiredRuntimeGitHandoff::present(ceiling.clone(), wrong, [1; 32]).is_err());
    let mut wrong = original.clone();
    wrong.expires_at += time::Duration::seconds(1);
    assert!(ExpiredRuntimeGitHandoff::present(ceiling.clone(), wrong, [1; 32]).is_err());
    let mut wrong = original.clone();
    wrong.binding_id = Uuid::nil();
    assert!(ExpiredRuntimeGitHandoff::present(ceiling.clone(), wrong, [1; 32]).is_err());
    let mut wrong = original;
    wrong.repository_id = RepositoryId::new(Uuid::nil());
    assert!(ExpiredRuntimeGitHandoff::present(ceiling, wrong, [1; 32]).is_err());
}
