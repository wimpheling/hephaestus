use crate::{artifacts::materialize_artifact, filesystem::make_tree_read_only};
use release_artifact_store::LocalArtifactStore;
use run_orchestrator::{RunRuntimeArtifact, RunRuntimeArtifactKind};
use sha2::{Digest, Sha256};
use std::{fs, os::unix::fs::PermissionsExt, process::Command};
use uuid::Uuid;

#[test]
fn materializes_verified_artifact_with_declared_mode() {
    let fixture = tempfile::tempdir().expect("fixture");
    let store = fixture.path().join("store");
    let release = fixture.path().join("release");
    fs::create_dir(&store).expect("store");
    fs::create_dir(&release).expect("release");
    let key = Uuid::new_v4();
    let bytes = b"exact release executable";
    fs::write(store.join(key.simple().to_string()), bytes).expect("object");
    let artifact = RunRuntimeArtifact {
        path: String::from("bin/agent"),
        kind: RunRuntimeArtifactKind::Executable,
        mode: 0o555,
        content_hash: Sha256::digest(bytes).into(),
        size_bytes: u64::try_from(bytes.len()).expect("length"),
        storage_key: key,
    };

    materialize_artifact(&store, &release, &artifact).expect("materialize");

    let output = release.join("bin/agent");
    assert_eq!(fs::read(&output).expect("output"), bytes);
    assert_eq!(
        fs::metadata(output).expect("metadata").permissions().mode() & 0o777,
        0o555
    );
}

#[test]
fn rejects_tampered_canonical_object() {
    let fixture = tempfile::tempdir().expect("fixture");
    let store = fixture.path().join("store");
    let release = fixture.path().join("release");
    fs::create_dir(&store).expect("store");
    fs::create_dir(&release).expect("release");
    let key = Uuid::new_v4();
    fs::write(store.join(key.simple().to_string()), b"tampered").expect("object");
    let artifact = RunRuntimeArtifact {
        path: String::from("agent"),
        kind: RunRuntimeArtifactKind::File,
        mode: 0o444,
        content_hash: [0; 32],
        size_bytes: 8,
        storage_key: key,
    };

    assert!(materialize_artifact(&store, &release, &artifact).is_err());
}

#[test]
fn executes_only_the_imported_read_only_release_artifact() {
    let fixture = tempfile::tempdir().expect("fixture");
    let store_root = fixture.path().join("store");
    let sealed_output = fixture.path().join("sealed-output");
    let source_tree = fixture.path().join("source");
    let release_tree = fixture.path().join("release");
    fs::create_dir(&store_root).expect("store");
    fs::set_permissions(&store_root, fs::Permissions::from_mode(0o700)).expect("store mode");
    fs::create_dir_all(sealed_output.join("bin")).expect("sealed output");
    fs::create_dir_all(source_tree.join("bin")).expect("source tree");
    fs::create_dir(&release_tree).expect("release tree");

    let built = sealed_output.join("bin/agent");
    fs::write(&built, b"#!/bin/sh\nprintf 'imported-release\\n'\n").expect("built executable");
    fs::set_permissions(&built, fs::Permissions::from_mode(0o755)).expect("built mode");
    let source_decoy = source_tree.join("bin/agent");
    fs::write(&source_decoy, b"#!/bin/sh\nexit 97\n").expect("source decoy");
    fs::set_permissions(&source_decoy, fs::Permissions::from_mode(0o755)).expect("source mode");

    let store = LocalArtifactStore::new(store_root.clone()).expect("artifact store");
    let imported = store
        .import_for(Uuid::new_v4(), &sealed_output)
        .expect("safe one-way import");
    assert_eq!(imported.len(), 1);
    let artifact = &imported[0];
    materialize_artifact(
        &store_root,
        &release_tree,
        &RunRuntimeArtifact {
            path: artifact.path.as_str().to_owned(),
            kind: RunRuntimeArtifactKind::Executable,
            mode: u32::from(artifact.mode),
            content_hash: *artifact.content_hash.as_bytes(),
            size_bytes: artifact.size_bytes,
            storage_key: artifact.storage_key,
        },
    )
    .expect("verified runtime materialization");
    make_tree_read_only(&release_tree).expect("seal release tree");

    let executable = release_tree.join("bin/agent");
    let output = Command::new(&executable)
        .current_dir(&source_tree)
        .output()
        .expect("execute imported release artifact");
    assert!(output.status.success());
    assert_eq!(output.stdout, b"imported-release\n");
    assert_eq!(
        fs::metadata(&executable)
            .expect("executable metadata")
            .permissions()
            .mode()
            & 0o777,
        0o555
    );
    assert_eq!(
        fs::metadata(&release_tree)
            .expect("release metadata")
            .permissions()
            .mode()
            & 0o777,
        0o555
    );
}
