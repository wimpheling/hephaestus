//! Cryptographic envelope and redaction tests.

use super::{
    EncryptedSecretVersion, EncryptedStore, LocalKeyProvider, SecretStoreError, VersionContext,
};
use forge_domain::ProjectId;
use secret_domain::{SecretId, SecretOwner, SecretValue, SecretVersionId};

const SENTINEL: &[u8] = b"top-secret-sentinel-cf58e2a4";

fn provider(active: &str) -> LocalKeyProvider {
    LocalKeyProvider::new(
        active,
        [
            ("key/v1", [1_u8; 32]),
            ("key/v2", [2_u8; 32]),
            ("wrong", [3_u8; 32]),
        ],
    )
    .expect("fixture keys should validate")
}

fn version_context(version_id: SecretVersionId) -> VersionContext {
    VersionContext {
        owner: SecretOwner::Project(ProjectId::new()),
        secret_id: SecretId::new(),
        version_id,
        sequence: 1,
        media_type: String::from("application/octet-stream"),
    }
}

fn sealed() -> (
    EncryptedStore<LocalKeyProvider>,
    VersionContext,
    EncryptedSecretVersion,
) {
    let store = EncryptedStore::new(provider("key/v1"));
    let context = version_context(SecretVersionId::new());
    let plaintext = SecretValue::new(SENTINEL).expect("sentinel should validate");
    let encrypted = store
        .seal(&context, &plaintext)
        .expect("encryption should succeed");
    (store, context, encrypted)
}

#[test]
fn envelope_round_trip_and_nonce_uniqueness() {
    let (store, context, first) = sealed();
    let plaintext = SecretValue::new(SENTINEL).expect("sentinel should validate");
    let second = store
        .seal(&version_context(SecretVersionId::new()), &plaintext)
        .expect("second encryption should succeed");
    assert_ne!(first.data_nonce, second.data_nonce);
    assert_ne!(first.wrap_nonce, second.wrap_nonce);
    assert_ne!(first.ciphertext, second.ciphertext);
    assert!(
        !first
            .ciphertext
            .windows(SENTINEL.len())
            .any(|v| v == SENTINEL)
    );
    assert!(
        !first
            .wrapped_data_key
            .windows(SENTINEL.len())
            .any(|value| value == SENTINEL)
    );

    let resolved = store
        .resolve(&context, &first)
        .expect("authorized exact context should resolve");
    assert_eq!(resolved.expose(), SENTINEL);
}

#[test]
fn ciphertext_nonce_and_associated_data_tampering_fail() {
    let (store, context, encrypted) = sealed();
    let mut mutations = Vec::new();
    let mut ciphertext = encrypted.clone();
    ciphertext.ciphertext[0] ^= 1;
    mutations.push(ciphertext);
    let mut data_nonce = encrypted.clone();
    data_nonce.data_nonce[0] ^= 1;
    mutations.push(data_nonce);
    let mut wrapped_key = encrypted.clone();
    wrapped_key.wrapped_data_key[0] ^= 1;
    mutations.push(wrapped_key);
    let mut wrap_nonce = encrypted.clone();
    wrap_nonce.wrap_nonce[0] ^= 1;
    mutations.push(wrap_nonce);
    let mut hash = encrypted;
    hash.associated_data_hash[0] ^= 1;
    mutations.push(hash);
    for mutation in mutations {
        assert!(store.resolve(&context, &mutation).is_err());
    }
    let mut wrong_context = context.clone();
    wrong_context.sequence = 2;
    assert!(
        store
            .resolve(&wrong_context, &mutations_fixture(&store, &context))
            .is_err()
    );
}

fn mutations_fixture(
    store: &EncryptedStore<LocalKeyProvider>,
    context: &VersionContext,
) -> EncryptedSecretVersion {
    store
        .seal(
            context,
            &SecretValue::new(SENTINEL).expect("sentinel should validate"),
        )
        .expect("fixture encryption should work")
}

#[test]
fn wrong_and_unavailable_keys_fail_closed() {
    let (_store, context, mut encrypted) = sealed();
    encrypted.key_reference = String::from("wrong");
    let wrong = EncryptedStore::new(provider("key/v1"));
    assert!(matches!(
        wrong.resolve(&context, &encrypted),
        Err(SecretStoreError::Authentication)
    ));

    let unavailable = LocalKeyProvider::new("key/v2", [("key/v2", [2_u8; 32])])
        .expect("remaining key should validate");
    let unavailable = EncryptedStore::new(unavailable);
    encrypted.key_reference = String::from("key/v1");
    assert!(matches!(
        unavailable.resolve(&context, &encrypted),
        Err(SecretStoreError::UnavailableKey)
    ));
}

#[test]
fn host_key_rotation_affects_only_later_versions_and_backups_restore() {
    let mut keys = provider("key/v1");
    let first_store = EncryptedStore::new(provider("key/v1"));
    let first_context = version_context(SecretVersionId::new());
    let first = first_store
        .seal(
            &first_context,
            &SecretValue::new(SENTINEL).expect("sentinel should validate"),
        )
        .expect("first encryption should work");

    keys.rotate_active_key("key/v2")
        .expect("known key should activate");
    let rotated = EncryptedStore::new(keys);
    let second_context = version_context(SecretVersionId::new());
    let second = rotated
        .seal(
            &second_context,
            &SecretValue::new(SENTINEL).expect("sentinel should validate"),
        )
        .expect("rotated encryption should work");
    assert_eq!(first.key_reference, "key/v1");
    assert_eq!(second.key_reference, "key/v2");
    assert_eq!(
        rotated
            .resolve(&first_context, &first)
            .expect("retained backup key should decrypt")
            .expose(),
        SENTINEL
    );
    assert_eq!(
        rotated
            .resolve(&second_context, &second)
            .expect("active key should decrypt")
            .expose(),
        SENTINEL
    );
}

#[test]
fn formatted_records_and_errors_never_expose_plaintext_or_keys() {
    let (_store, _context, encrypted) = sealed();
    let error = SecretStoreError::Authentication;
    for rendered in [
        format!("{encrypted:?}"),
        format!("{error:?}"),
        error.to_string(),
    ] {
        assert!(!rendered.contains(std::str::from_utf8(SENTINEL).expect("sentinel is UTF-8")));
        assert!(!rendered.contains(&"01".repeat(32)));
    }
}
