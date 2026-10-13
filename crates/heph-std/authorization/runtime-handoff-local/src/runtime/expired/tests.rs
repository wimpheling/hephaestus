//! Actual encrypted-filesystem behavior; controlled models are not PG provenance.

use super::super::{EncryptedFileHandoffStore, StoredEnvelope};
use crate::expired_test_support::generic;
use capability_domain::{RuntimeCredentialGeneration, RuntimeCredentialHash, RuntimeSessionId};
use runtime_authority::{RuntimeAuthorityError, RuntimeHandoffStore};
use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt},
};
use time::{Duration, OffsetDateTime};

#[test]
fn only_expired_original_is_removed_other_envelope_and_inode_remain_and_retry_is_exact() {
    let directory = tempfile::tempdir().unwrap();
    let store = EncryptedFileHandoffStore::new(directory.path(), [31; 32]).unwrap();
    let a = RuntimeSessionId::new();
    let b = RuntimeSessionId::new();
    let generation = RuntimeCredentialGeneration::INITIAL;
    let expires = OffsetDateTime::UNIX_EPOCH + Duration::hours(1);
    let credential = store.create(a, generation, expires).unwrap();
    let verifier = credential.storage_hash(a, generation);
    drop(credential);
    let other = store
        .create(b, generation, expires + Duration::hours(1))
        .unwrap();
    let b_path = store.path(b, generation);
    let b_bytes = fs::read(&b_path).unwrap();
    let b_inode = fs::metadata(&b_path).unwrap().ino();
    let original = generic(a, generation, expires, verifier);
    assert_eq!(
        store.open(a, generation, expires).unwrap_err(),
        RuntimeAuthorityError::HandoffUnavailable
    );
    store.destroy_expired(&original).unwrap();
    assert!(!store.path(a, generation).exists());
    assert_eq!(fs::read(&b_path).unwrap(), b_bytes);
    assert_eq!(fs::metadata(&b_path).unwrap().ino(), b_inode);
    assert!(store.open(b, generation, expires).unwrap().expose() == other.expose());
    drop(other);
    store.destroy_expired(&original).unwrap();
    assert_eq!(fs::read(&b_path).unwrap(), b_bytes);
}

#[test]
fn wrong_key_expiry_and_verifier_preserve_original_bytes_and_inode() {
    let directory = tempfile::tempdir().unwrap();
    let store = EncryptedFileHandoffStore::new(directory.path(), [32; 32]).unwrap();
    let id = RuntimeSessionId::new();
    let generation = RuntimeCredentialGeneration::INITIAL;
    let expires = OffsetDateTime::UNIX_EPOCH + Duration::hours(1);
    let credential = store.create(id, generation, expires).unwrap();
    let verifier = credential.storage_hash(id, generation);
    drop(credential);
    let original = generic(id, generation, expires, verifier);
    let path = store.path(id, generation);
    let bytes = fs::read(&path).unwrap();
    let inode = fs::metadata(&path).unwrap().ino();
    let wrong = EncryptedFileHandoffStore::new(directory.path(), [33; 32]).unwrap();
    assert!(wrong.destroy_expired(&original).is_err());
    drop(wrong);
    let wrong_expiry = generic(id, generation, expires + Duration::nanoseconds(1), verifier);
    assert!(store.destroy_expired(&wrong_expiry).is_err());
    let wrong_verifier = generic(
        id,
        generation,
        expires,
        RuntimeCredentialHash::from_bytes([0; 32]),
    );
    assert!(store.destroy_expired(&wrong_verifier).is_err());
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert_eq!(fs::metadata(&path).unwrap().ino(), inode);
}

#[test]
fn wrong_generation_envelope_cannot_authenticate_under_a_different_exact_filename() {
    let directory = tempfile::tempdir().unwrap();
    let store = EncryptedFileHandoffStore::new(directory.path(), [34; 32]).unwrap();
    let id = RuntimeSessionId::new();
    let first = RuntimeCredentialGeneration::INITIAL;
    let second = RuntimeCredentialGeneration::new(2).unwrap();
    let expires = OffsetDateTime::UNIX_EPOCH + Duration::hours(1);
    let credential = store.create(id, first, expires).unwrap();
    let verifier = credential.storage_hash(id, second);
    drop(credential);
    let bytes = fs::read(store.path(id, first)).unwrap();
    let substituted = store.path(id, second);
    fs::write(&substituted, &bytes).unwrap();
    fs::set_permissions(&substituted, fs::Permissions::from_mode(0o600)).unwrap();
    let original = generic(id, second, expires, verifier);
    assert!(store.destroy_expired(&original).is_err());
    assert_eq!(fs::read(substituted).unwrap(), bytes);
    assert_eq!(fs::read(store.path(id, first)).unwrap(), bytes);
}

#[test]
fn unknown_algorithm_nonce_ciphertext_and_expiry_corruption_are_preserved() {
    for corruption in 0..5 {
        let directory = tempfile::tempdir().unwrap();
        let store = EncryptedFileHandoffStore::new(directory.path(), [35; 32]).unwrap();
        let id = RuntimeSessionId::new();
        let generation = RuntimeCredentialGeneration::INITIAL;
        let expires = OffsetDateTime::UNIX_EPOCH + Duration::hours(1);
        let credential = store.create(id, generation, expires).unwrap();
        let verifier = credential.storage_hash(id, generation);
        drop(credential);
        let original = generic(id, generation, expires, verifier);
        let path = store.path(id, generation);
        let mut envelope: StoredEnvelope =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        match corruption {
            0 => envelope.algorithm = "unknown".into(),
            1 => envelope.nonce[0] ^= 1,
            2 => envelope.ciphertext[0] ^= 1,
            3 => envelope.expires_at_unix_nanos += 1,
            _ => {}
        }
        let changed = if corruption == 4 {
            let mut value = serde_json::to_value(&envelope).unwrap();
            value
                .as_object_mut()
                .unwrap()
                .insert("unknown".into(), serde_json::Value::Bool(true));
            serde_json::to_vec(&value).unwrap()
        } else {
            serde_json::to_vec(&envelope).unwrap()
        };
        fs::write(&path, &changed).unwrap();
        let inode = fs::metadata(&path).unwrap().ino();
        assert!(store.destroy_expired(&original).is_err());
        assert_eq!(fs::read(&path).unwrap(), changed);
        assert_eq!(fs::metadata(&path).unwrap().ino(), inode);
    }
}

#[test]
fn replaced_root_and_symlink_envelope_deny_through_retained_store() {
    let parent = tempfile::tempdir().unwrap();
    let path = parent.path().join("original-root");
    let store = EncryptedFileHandoffStore::new(&path, [36; 32]).unwrap();
    let id = RuntimeSessionId::new();
    let generation = RuntimeCredentialGeneration::INITIAL;
    let expires = OffsetDateTime::UNIX_EPOCH + Duration::hours(1);
    let credential = store.create(id, generation, expires).unwrap();
    let verifier = credential.storage_hash(id, generation);
    drop(credential);
    let original = generic(id, generation, expires, verifier);
    let target = store.path(id, generation);
    let bytes = fs::read(&target).unwrap();
    let retained = path.join("retained-envelope");
    fs::rename(&target, &retained).unwrap();
    std::os::unix::fs::symlink(&retained, &target).unwrap();
    assert!(store.destroy_expired(&original).is_err());
    assert_eq!(fs::read(&retained).unwrap(), bytes);
    fs::remove_file(&target).unwrap();
    fs::rename(&retained, &target).unwrap();
    let held = parent.path().join("held-root");
    fs::rename(&path, &held).unwrap();
    fs::create_dir(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(&target, &bytes).unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).unwrap();
    assert!(store.destroy_expired(&original).is_err());
    assert_eq!(fs::read(&target).unwrap(), bytes);
    assert_eq!(
        fs::read(held.join(target.file_name().unwrap())).unwrap(),
        bytes
    );
}
