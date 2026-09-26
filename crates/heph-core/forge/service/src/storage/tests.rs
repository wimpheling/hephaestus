use super::{
    GitStorage,
    read::{valid_commit, valid_relative_path},
};
use forge_domain::RepositoryId;
use std::{fs, process::Command};

fn git(directory: &std::path::Path, arguments: &[&str]) {
    let status = Command::new("git")
        .args(arguments)
        .current_dir(directory)
        .status()
        .expect("run Git");
    assert!(status.success(), "Git command failed: {arguments:?}");
}

#[tokio::test]
async fn derives_only_canonical_opaque_paths() {
    let temporary = tempfile::tempdir().expect("temporary directory");
    let storage = GitStorage::initialize(temporary.path())
        .await
        .expect("storage");
    let id = RepositoryId::new();
    assert_eq!(
        storage.repository_path(id),
        temporary
            .path()
            .canonicalize()
            .expect("canonical")
            .join(format!("{id}.git"))
    );
    assert_eq!(GitStorage::parse_route(&id.to_string()).expect("id"), id);
    assert!(GitStorage::parse_route("../etc").is_err());
    assert!(GitStorage::parse_route(&format!("{id}/../../etc")).is_err());
    assert!(GitStorage::parse_route(&id.to_string().replace('-', "")).is_err());
}

#[tokio::test]
async fn creates_and_validates_bare_repository() {
    let temporary = tempfile::tempdir().expect("temporary directory");
    let storage = GitStorage::initialize(temporary.path())
        .await
        .expect("storage");
    let id = RepositoryId::new();
    let path = storage
        .create_bare(id, "main")
        .await
        .expect("bare repository");
    assert_eq!(storage.validate_existing(id).await.expect("existing"), path);
}

#[tokio::test]
async fn reads_a_bounded_blob_from_the_exact_commit() {
    let storage_root = tempfile::tempdir().expect("storage root");
    let worktree = tempfile::tempdir().expect("worktree");
    let storage = GitStorage::initialize(storage_root.path())
        .await
        .expect("storage");
    let repository_id = RepositoryId::new();
    let bare = storage
        .create_bare(repository_id, "main")
        .await
        .expect("bare repository");
    git(worktree.path(), &["init", "--initial-branch=main"]);
    git(
        worktree.path(),
        &["config", "user.email", "test@example.invalid"],
    );
    git(worktree.path(), &["config", "user.name", "Storage Test"]);
    fs::write(worktree.path().join("heph.gateways.toml"), b"version = 1\n").expect("manifest");
    git(worktree.path(), &["add", "heph.gateways.toml"]);
    git(worktree.path(), &["commit", "-m", "manifest"]);
    let commit = String::from_utf8(
        Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(worktree.path())
            .output()
            .expect("resolve commit")
            .stdout,
    )
    .expect("commit text")
    .trim()
    .to_owned();
    git(
        worktree.path(),
        &["remote", "add", "origin", bare.to_str().expect("bare path")],
    );
    git(worktree.path(), &["push", "origin", "main"]);
    fs::write(worktree.path().join("heph.gateways.toml"), b"version = 2\n")
        .expect("updated manifest");
    git(worktree.path(), &["add", "heph.gateways.toml"]);
    git(worktree.path(), &["commit", "-m", "updated manifest"]);
    git(worktree.path(), &["push", "origin", "main"]);

    assert_eq!(
        storage
            .read_file_at_commit(repository_id, &commit, "heph.gateways.toml", 64)
            .await
            .expect("read exact blob"),
        b"version = 1\n"
    );
    assert!(
        storage
            .read_file_at_commit(repository_id, &commit, "heph.gateways.toml", 4)
            .await
            .is_err()
    );
}

#[test]
fn accepts_only_full_lowercase_commit_names_and_relative_paths() {
    assert!(valid_commit(&"a".repeat(40)));
    assert!(valid_commit(&"b".repeat(64)));
    assert!(!valid_commit(&"A".repeat(40)));
    assert!(!valid_commit(&"a".repeat(39)));
    assert!(valid_relative_path("heph.gateways.toml"));
    assert!(valid_relative_path("config/heph.gateways.toml"));
    assert!(!valid_relative_path("../heph.gateways.toml"));
    assert!(!valid_relative_path("/heph.gateways.toml"));
}
