//! Genuine encrypted Git envelopes with controlled, non-PG comparison models.

use super::super::{EncryptedFileRuntimeGitHandoffStore, StoredEnvelope};
use crate::expired_test_support::{git, no_git};
use capability_domain::{RuntimeCredentialGeneration, RuntimeSessionId};
use runtime_git_authority::{RuntimeGitAuthorityError, RuntimeGitHandoffStore};
use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt},
};
use time::{Duration, OffsetDateTime};

#[test]
fn present_git_removes_only_exact_expired_original_and_replays_without_touching_other() {
    let directory = tempfile::tempdir().unwrap();
    let store = EncryptedFileRuntimeGitHandoffStore::new(directory.path(), [41; 32]).unwrap();
    let a = RuntimeSessionId::new();
    let b = RuntimeSessionId::new();
    let generation = RuntimeCredentialGeneration::INITIAL;
    let expires = OffsetDateTime::UNIX_EPOCH + Duration::hours(1);
    let first = store.create(a, generation, expires).unwrap();
    let verifier = first.storage_hash().as_bytes();
    drop(first);
    let other = store
        .create(b, generation, expires + Duration::hours(1))
        .unwrap();
    let original = git(a, generation, expires, verifier);
    let b_path = store.path(b, generation);
    let bytes = fs::read(&b_path).unwrap();
    let inode = fs::metadata(&b_path).unwrap().ino();
    assert_eq!(
        store.open(a, generation, expires).unwrap_err(),
        RuntimeGitAuthorityError::HandoffUnavailable
    );
    store.destroy_expired(&original).unwrap();
    store.destroy_expired(&original).unwrap();
    assert!(!store.path(a, generation).exists());
    assert_eq!(fs::read(&b_path).unwrap(), bytes);
    assert_eq!(fs::metadata(&b_path).unwrap().ino(), inode);
    assert!(store.open(b, generation, expires).unwrap().expose() == other.expose());
    drop(other);
}

#[test]
fn qualified_no_git_absence_never_deletes_an_unexpected_exact_file() {
    let directory = tempfile::tempdir().unwrap();
    let store = EncryptedFileRuntimeGitHandoffStore::new(directory.path(), [42; 32]).unwrap();
    let id = RuntimeSessionId::new();
    let generation = RuntimeCredentialGeneration::INITIAL;
    let expires = OffsetDateTime::UNIX_EPOCH + Duration::hours(1);
    let absent = no_git(id, generation, expires);
    store.destroy_expired(&absent).unwrap();
    store.create(id, generation, expires).unwrap();
    let path = store.path(id, generation);
    let bytes = fs::read(&path).unwrap();
    let inode = fs::metadata(&path).unwrap().ino();
    assert!(store.destroy_expired(&absent).is_err());
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert_eq!(fs::metadata(&path).unwrap().ino(), inode);
}

#[test]
fn git_wrong_key_expiry_verifier_algorithm_nonce_and_ciphertext_all_preserve_evidence() {
    for corruption in 0..6 {
        let directory = tempfile::tempdir().unwrap();
        let store = EncryptedFileRuntimeGitHandoffStore::new(directory.path(), [43; 32]).unwrap();
        let id = RuntimeSessionId::new();
        let generation = RuntimeCredentialGeneration::INITIAL;
        let expires = OffsetDateTime::UNIX_EPOCH + Duration::hours(1);
        let first = store.create(id, generation, expires).unwrap();
        let verifier = first.storage_hash().as_bytes();
        drop(first);
        let path = store.path(id, generation);
        let original = git(id, generation, expires, verifier);
        if corruption >= 3 {
            let mut envelope: StoredEnvelope =
                serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            match corruption {
                3 => envelope.algorithm = "unknown".into(),
                4 => envelope.nonce[0] ^= 1,
                _ => envelope.ciphertext[0] ^= 1,
            }
            fs::write(&path, serde_json::to_vec(&envelope).unwrap()).unwrap();
        }
        let bytes = fs::read(&path).unwrap();
        let inode = fs::metadata(&path).unwrap().ino();
        let result = match corruption {
            0 => EncryptedFileRuntimeGitHandoffStore::new(directory.path(), [44; 32])
                .unwrap()
                .destroy_expired(&original),
            1 => store.destroy_expired(&git(
                id,
                generation,
                expires + Duration::nanoseconds(1),
                verifier,
            )),
            2 => store.destroy_expired(&git(id, generation, expires, [0; 32])),
            _ => store.destroy_expired(&original),
        };
        assert!(result.is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert_eq!(fs::metadata(&path).unwrap().ino(), inode);
    }
}

#[test]
fn no_git_absence_on_replaced_empty_root_is_denied_instead_of_reacquired() {
    let parent = tempfile::tempdir().unwrap();
    let path = parent.path().join("git-root");
    let store = EncryptedFileRuntimeGitHandoffStore::new(&path, [45; 32]).unwrap();
    let original = no_git(
        RuntimeSessionId::new(),
        RuntimeCredentialGeneration::INITIAL,
        OffsetDateTime::UNIX_EPOCH + Duration::hours(1),
    );
    let held = parent.path().join("held-root");
    fs::rename(&path, &held).unwrap();
    fs::create_dir(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(store.destroy_expired(&original).is_err());
    assert!(held.is_dir());
    assert!(path.is_dir());
}
