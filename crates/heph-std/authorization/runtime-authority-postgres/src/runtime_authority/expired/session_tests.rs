//! Controlled serializer/row denials; no raw database positive fixture.

use super::super::original::controlled_original;
use super::*;
use capability_domain::{AuthorizationSnapshotId, WorkloadKind, WorkloadPrincipal};

fn row() -> (Original, AuthorizationSnapshot, Session) {
    let original = controlled_original();
    let snapshot = AuthorizationSnapshot::new(
        AuthorizationSnapshotId::from_uuid(Uuid::from_u128(9)),
        WorkloadPrincipal::new(
            WorkloadKind::AgentInstance,
            original.instance_id,
            original.revision_id,
        ),
        "controlled-unit-row",
        vec![],
    )
    .unwrap();
    let issued = OffsetDateTime::UNIX_EPOCH;
    let expires = issued + time::Duration::seconds(60);
    let identity = RuntimeSessionIdentity::new(
        RuntimeSessionId::from_uuid(Uuid::from_u128(10)),
        snapshot.principal(),
        RuntimeInvocation::Run(original.id.into()),
        &snapshot,
        issued,
        expires,
    )
    .unwrap();
    let row = Session {
        id: identity.id().as_uuid(),
        snapshot_id: snapshot.id().as_uuid(),
        run_id: original.id,
        instance_id: original.instance_id,
        revision_id: original.revision_id,
        attachment: original.attachment,
        identity_hash: identity.normalized_hash().as_bytes().to_vec(),
        snapshot_hash: snapshot.normalized_hash().as_bytes().to_vec(),
        credential_hash: vec![7; 32],
        issuance_generation: 1,
        status: "expired".into(),
        issued_at: issued,
        expires_at: expires,
        acknowledged_at: None,
        revoked_at: None,
        revocation_reason: None,
    };
    (original, snapshot, row)
}

#[test]
fn wrong_session_run_attachment_and_generation_deny() {
    let (original, _, mut row) = row();
    let id = RuntimeSessionId::from_uuid(row.id);
    assert_eq!(
        row.check(&original, id, RuntimeCredentialGeneration::new(2).unwrap()),
        Err(RuntimeAuthorityError::GenerationMismatch)
    );
    row.run_id = Uuid::from_u128(99);
    assert!(
        row.check(&original, id, RuntimeCredentialGeneration::INITIAL)
            .is_err()
    );
    row.run_id = original.id;
    row.attachment = None;
    assert!(
        row.check(&original, id, RuntimeCredentialGeneration::INITIAL)
            .is_err()
    );
    row.attachment = original.attachment;
    assert!(
        row.check(
            &original,
            RuntimeSessionId::from_uuid(Uuid::from_u128(99)),
            RuntimeCredentialGeneration::INITIAL
        )
        .is_err()
    );
}

#[test]
fn active_revoked_reason_and_malformed_verifier_deny() {
    let (original, _, mut row) = row();
    let id = RuntimeSessionId::from_uuid(row.id);
    row.status = "active".into();
    assert!(
        row.check(&original, id, RuntimeCredentialGeneration::INITIAL)
            .is_err()
    );
    row.status = "expired".into();
    row.revocation_reason = Some("contradictory".into());
    assert!(
        row.check(&original, id, RuntimeCredentialGeneration::INITIAL)
            .is_err()
    );
    row.revocation_reason = None;
    row.credential_hash.pop();
    assert!(
        row.check(&original, id, RuntimeCredentialGeneration::INITIAL)
            .is_err()
    );
}

#[test]
fn lost_original_nanoseconds_are_denied_without_guessing_or_rounding_hashes() {
    let (original, snapshot, mut row) = row();
    // Original ordinary producer hashed these nanosecond claims. The simulated
    // loaded PG timestamps lost sub-microsecond information; this is not PG evidence.
    let identity = RuntimeSessionIdentity::new(
        RuntimeSessionId::from_uuid(row.id),
        snapshot.principal(),
        RuntimeInvocation::Run(original.id.into()),
        &snapshot,
        row.issued_at + time::Duration::nanoseconds(123),
        row.expires_at + time::Duration::nanoseconds(123),
    )
    .unwrap();
    row.identity_hash = identity.normalized_hash().as_bytes().to_vec();
    let observed = row.expires_at;
    assert_eq!(
        row.project(&original, snapshot, observed).unwrap_err(),
        RuntimeAuthorityError::IdentityMismatch
    );
}

#[test]
fn wrong_snapshot_hash_and_early_observation_deny() {
    let (original, snapshot, mut record) = row();
    record.snapshot_hash[0] ^= 1;
    let observed = record.expires_at;
    assert!(record.project(&original, snapshot, observed).is_err());
    let (original, snapshot, record) = row();
    let observed = record.expires_at - time::Duration::seconds(1);
    assert!(record.project(&original, snapshot, observed).is_err());
}
