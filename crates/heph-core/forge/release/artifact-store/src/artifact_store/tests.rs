use super::{ArtifactStoreError, LocalArtifactStore, MAX_ARTIFACT_BYTES, MAX_ARTIFACT_FILES};
use release_domain::ContentHash;
use std::{
    fs::{self, hard_link},
    os::unix::{
        fs::{PermissionsExt, symlink},
        net::UnixListener,
    },
    process::Command,
};
use uuid::Uuid;

fn fixture() -> (tempfile::TempDir, LocalArtifactStore) {
    let temporary = tempfile::tempdir().expect("temporary fixture");
    let store = temporary.path().join("store");
    fs::create_dir(&store).expect("store root");
    fs::set_permissions(&store, fs::Permissions::from_mode(0o700)).expect("store mode");
    (
        temporary,
        LocalArtifactStore::new(store).expect("valid store"),
    )
}

#[test]
fn imports_deterministic_manifest_and_immutable_objects() {
    let (temporary, store) = fixture();
    let output = temporary.path().join("output");
    fs::create_dir_all(output.join("bin")).expect("output directories");
    fs::write(output.join("README"), b"documentation").expect("readme");
    fs::write(output.join("bin/reviewer"), b"exact executable").expect("executable");
    fs::set_permissions(
        output.join("bin/reviewer"),
        fs::Permissions::from_mode(0o755),
    )
    .expect("executable mode");

    let manifest = store.import(&output).expect("safe import");
    assert_eq!(
        manifest
            .iter()
            .map(|artifact| artifact.path.as_str())
            .collect::<Vec<_>>(),
        ["README", "bin/reviewer"]
    );
    assert_eq!(manifest[0].mode, 0o444);
    assert_eq!(manifest[1].mode, 0o555);
    for artifact in manifest {
        let path = store.resolve(artifact.storage_key).expect("stored object");
        assert_eq!(
            fs::metadata(path)
                .expect("object metadata")
                .permissions()
                .mode()
                & 0o777,
            0o400
        );
    }
}

#[test]
fn stable_import_reuses_verified_canonical_objects() {
    let (temporary, store) = fixture();
    let output = temporary.path().join("output");
    fs::create_dir(&output).expect("output");
    fs::write(output.join("agent"), b"stable bytes").expect("artifact");
    let operation_id = Uuid::new_v4();

    let first = store
        .import_for(operation_id, &output)
        .expect("first import");
    let second = store
        .import_for(operation_id, &output)
        .expect("retry import");

    assert_eq!(first, second);
}

#[test]
fn read_verified_returns_owned_hash_verified_bytes_at_limit() {
    let (temporary, store) = fixture();
    let key = Uuid::new_v4();
    let bytes = b"verified artifact";
    let object = temporary
        .path()
        .join("store")
        .join(key.simple().to_string());
    fs::write(&object, bytes).expect("canonical object");

    let returned = store
        .read_verified(
            key,
            ContentHash::digest(bytes),
            u64::try_from(bytes.len()).expect("fixture length"),
            u64::try_from(bytes.len()).expect("fixture length"),
        )
        .expect("verified read");
    fs::write(object, b"changed after read").expect("mutate fixture object");
    assert_eq!(returned, bytes);
}

#[test]
fn read_verified_rejects_hash_length_and_limit_mismatches() {
    let (temporary, store) = fixture();
    let key = Uuid::new_v4();
    let bytes = b"tampered";
    let object = temporary
        .path()
        .join("store")
        .join(key.simple().to_string());
    fs::write(&object, bytes).expect("canonical object");
    let size = u64::try_from(bytes.len()).expect("fixture length");

    assert_eq!(
        store.read_verified(key, ContentHash::digest(b"expected"), size, size),
        Err(ArtifactStoreError::ObjectConflict)
    );
    assert_eq!(
        store.read_verified(key, ContentHash::digest(bytes), size + 1, size + 1),
        Err(ArtifactStoreError::ObjectConflict)
    );
    assert_eq!(
        store.read_verified(key, ContentHash::digest(bytes), size, size - 1),
        Err(ArtifactStoreError::ReadLimit)
    );
}

#[test]
fn read_verified_rejects_symlink_and_hardlink_objects() {
    let (temporary, store) = fixture();
    let bytes = b"canonical bytes";
    let outside = temporary.path().join("outside");
    fs::write(&outside, bytes).expect("outside object");
    let symlink_key = Uuid::new_v4();
    symlink(
        &outside,
        temporary
            .path()
            .join("store")
            .join(symlink_key.simple().to_string()),
    )
    .expect("object symlink");
    let size = u64::try_from(bytes.len()).expect("fixture length");
    assert!(
        store
            .read_verified(symlink_key, ContentHash::digest(bytes), size, size)
            .is_err()
    );

    let hardlink_key = Uuid::new_v4();
    let object = temporary
        .path()
        .join("store")
        .join(hardlink_key.simple().to_string());
    fs::write(&object, bytes).expect("hardlink object");
    hard_link(&object, temporary.path().join("store/alias")).expect("hardlink alias");
    assert_eq!(
        store.read_verified(hardlink_key, ContentHash::digest(bytes), size, size),
        Err(ArtifactStoreError::UnsafeObject)
    );
}

#[cfg(target_os = "linux")]
#[test]
fn read_verified_rejects_fifo_without_blocking() {
    let (temporary, store) = fixture();
    let key = Uuid::new_v4();
    let fifo = temporary
        .path()
        .join("store")
        .join(key.simple().to_string());
    assert!(
        Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .expect("execute mkfifo")
            .success(),
        "create FIFO"
    );
    assert_eq!(
        store.read_verified(key, ContentHash::digest(b""), 0, 0),
        Err(ArtifactStoreError::UnsafeObject)
    );
}

#[test]
fn rejects_symlinks_and_hardlinks() {
    let (temporary, store) = fixture();
    let output = temporary.path().join("output");
    fs::create_dir(&output).expect("output");
    symlink("/etc/passwd", output.join("escape")).expect("symlink");
    assert!(matches!(
        store.import(&output),
        Err(ArtifactStoreError::UnsafeObject)
    ));
    fs::remove_file(output.join("escape")).expect("remove symlink");
    fs::write(output.join("one"), b"aliased").expect("source");
    hard_link(output.join("one"), output.join("two")).expect("hard link");
    assert!(matches!(
        store.import(&output),
        Err(ArtifactStoreError::HardLink)
    ));
}

#[test]
fn rejects_fifos_sockets_and_special_permission_modes() {
    let (temporary, store) = fixture();
    assert_eq!(
        store.import(std::path::Path::new("/dev/null")),
        Err(ArtifactStoreError::InvalidSourceRoot),
        "a character device cannot be used as an import root"
    );
    let output = temporary.path().join("output");
    fs::create_dir(&output).expect("output");

    let fifo = output.join("fifo");
    assert!(
        Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .expect("execute mkfifo")
            .success(),
        "create FIFO"
    );
    assert_eq!(store.import(&output), Err(ArtifactStoreError::UnsafeObject));
    fs::remove_file(&fifo).expect("remove FIFO");

    let socket_path = output.join("socket");
    let listener = UnixListener::bind(&socket_path).expect("create Unix socket");
    assert_eq!(store.import(&output), Err(ArtifactStoreError::UnsafeObject));
    drop(listener);
    fs::remove_file(&socket_path).expect("remove Unix socket");

    let privileged = output.join("setuid");
    fs::write(&privileged, b"ordinary bytes").expect("write setuid fixture");
    fs::set_permissions(&privileged, fs::Permissions::from_mode(0o4755)).expect("set special mode");
    assert_eq!(store.import(&output), Err(ArtifactStoreError::UnsafeMode));
}

#[test]
fn rejects_the_first_values_over_file_and_byte_quotas() {
    let (temporary, store) = fixture();
    let output = temporary.path().join("output");
    fs::create_dir(&output).expect("output");
    for index in 0..=MAX_ARTIFACT_FILES {
        fs::File::create(output.join(format!("artifact-{index:04}")))
            .expect("create count-boundary artifact");
    }
    assert_eq!(store.import(&output), Err(ArtifactStoreError::FileCount));

    fs::remove_dir_all(&output).expect("remove count fixture");
    fs::create_dir(&output).expect("recreate output");
    let oversized = fs::File::create(output.join("oversized")).expect("oversized fixture");
    oversized
        .set_len(MAX_ARTIFACT_BYTES + 1)
        .expect("create sparse file over byte quota");
    assert_eq!(store.import(&output), Err(ArtifactStoreError::TotalSize));
}
