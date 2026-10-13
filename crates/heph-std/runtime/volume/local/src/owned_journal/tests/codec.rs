use super::super::{
    JournalError, JournalPurpose,
    codec::{self, Kind, MAX_RECORD_BYTES},
    filesystem::FileIdentity,
    records::Birth,
};
use super::support::Fixture;
use std::path::Path;

#[test]
fn every_truncated_or_trailing_record_is_rejected_without_host_actions() {
    let bytes = codec::encode(Kind::Birth, b"canonical payload").expect("record");
    assert_eq!(
        codec::decode(Kind::Birth, &bytes).expect("valid decode"),
        b"canonical payload"
    );
    for end in 0..bytes.len() {
        assert!(codec::decode(Kind::Birth, &bytes[..end]).is_err());
    }
    let mut trailing = bytes.clone();
    trailing.push(0);
    assert!(codec::decode(Kind::Birth, &trailing).is_err());
    let mut corrupted = bytes;
    corrupted[13] ^= 1;
    assert!(codec::decode(Kind::Birth, &corrupted).is_err());
    assert!(codec::decode(Kind::Birth, &vec![0; MAX_RECORD_BYTES + 1]).is_err());
}

#[test]
fn valid_checksum_does_not_permit_another_kind_or_unknown_payload_fields() {
    let fixture = Fixture::new();
    let identity = FileIdentity {
        device: 1,
        inode: 2,
    };
    let bytes = Birth::encode(fixture.purpose.bytes(), identity, identity).expect("birth");
    let payload = codec::decode(Kind::Birth, &bytes).expect("payload");
    let wrong_kind = codec::encode(Kind::Attempt, payload).expect("other kind");
    assert!(Birth::decode(&wrong_kind, fixture.purpose.bytes()).is_err());
    let mut unknown_fields = payload.to_vec();
    unknown_fields.push(0);
    let bytes = codec::encode(Kind::Birth, &unknown_fields).expect("unknown payload fields");
    assert!(Birth::decode(&bytes, fixture.purpose.bytes()).is_err());
}

#[test]
fn checked_purpose_binds_full_receipt_host_root_and_birth_generation() {
    let fixture = Fixture::new();
    assert_eq!(fixture.purpose.generation(), 1);
    let changed = JournalPurpose::new(
        &fixture.receipt,
        "another-host",
        fixture.root.path(),
        fixture.owner_namespace,
        1,
    )
    .expect("changed purpose");
    assert_ne!(fixture.purpose.bytes(), changed.bytes());
    let changed = JournalPurpose::new(
        &fixture.receipt,
        "host-fixture",
        fixture.root.path(),
        fixture.owner_namespace,
        2,
    )
    .expect("changed generation");
    assert_ne!(fixture.purpose.bytes(), changed.bytes());
    let mut forged = fixture.receipt.clone();
    forged.registration_hash = [0; 32];
    assert!(
        JournalPurpose::new(
            &forged,
            "host-fixture",
            fixture.root.path(),
            fixture.owner_namespace,
            1
        )
        .is_err()
    );
    assert!(
        JournalPurpose::new(
            &fixture.receipt,
            "host-fixture",
            fixture.root.path(),
            fixture.owner_namespace,
            0
        )
        .is_err()
    );
    assert!(matches!(
        JournalPurpose::new(
            &fixture.receipt,
            "host-fixture",
            Path::new("/host-root/../alias"),
            fixture.owner_namespace,
            1
        ),
        Err(JournalError::Conflict(_))
    ));
    // Construction and record decoding accept no host handles and perform no IO.
    assert!(
        JournalPurpose::new(
            &fixture.receipt,
            "host-fixture",
            Path::new("/nonexistent-purpose-only-root"),
            fixture.owner_namespace,
            1
        )
        .is_ok()
    );
}

#[test]
fn physical_owner_namespace_changes_purpose_and_matches_core_encoding() {
    let fixture = Fixture::new();
    let changed = volume_trait::VolumeRootNamespaceId::from_uuid(uuid::Uuid::new_v4()).unwrap();
    let changed = JournalPurpose::new(
        &fixture.receipt,
        "host-fixture",
        fixture.root.path(),
        changed,
        1,
    )
    .unwrap();
    assert_ne!(fixture.purpose.bytes(), changed.bytes());
    let checked = volume_trait::OwnedBackingPurpose::new(
        fixture.receipt.clone(),
        String::from("host-fixture"),
        fixture.root.path().to_path_buf(),
        fixture.owner_namespace,
        1,
    )
    .unwrap();
    assert_eq!(fixture.purpose.bytes(), checked.canonical_bytes());
}
