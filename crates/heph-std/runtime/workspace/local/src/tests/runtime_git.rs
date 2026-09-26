use super::super::*;
use crate::{common::RuntimeGitWorkspaceRequest, runtime_materialize::materialize_runtime_git};
use std::{collections::BTreeSet, fs, os::unix::fs::MetadataExt, path::Path, process::Command};
use uuid::Uuid;

#[test]
fn runtime_git_materialization_is_private_and_remote_free() {
    let temporary = tempfile::tempdir().expect("temporary runtime Git root");
    let source = temporary.path().join("source");
    let repository = temporary.path().join("repository.git");
    let workspace_root = temporary.path().join("workspaces");
    let active_root = workspace_root.join("active");
    let staging = active_root.join("staging");
    let active = active_root.join(Uuid::new_v4().to_string());
    fs::create_dir(&source).expect("source directory");
    fs::create_dir_all(&active_root).expect("active directory");
    run_git(&source, &["init", "--initial-branch=main"]);
    run_git(&source, &["config", "user.name", "Runtime test"]);
    run_git(
        &source,
        &["config", "user.email", "runtime@example.invalid"],
    );
    fs::write(source.join("session.txt"), "immutable\n").expect("source file");
    run_git(&source, &["add", "."]);
    run_git(&source, &["commit", "-m", "initial"]);
    fs::write(source.join("transcript.txt"), "second\n").expect("second source file");
    run_git(&source, &["add", "."]);
    run_git(&source, &["commit", "-m", "second"]);
    let parent = git_output_test(&source, &["rev-parse", "HEAD"]);
    fs::write(source.join("response.txt"), "third\n").expect("third source file");
    run_git(&source, &["add", "."]);
    run_git(&source, &["commit", "-m", "third"]);
    let commit = git_output_test(&source, &["rev-parse", "HEAD"]);
    run_git(
        &source,
        &[
            "init",
            "--bare",
            repository.to_str().expect("repository path"),
        ],
    );
    run_git(
        &source,
        &[
            "push",
            repository.to_str().expect("repository path"),
            "HEAD:refs/heads/main",
        ],
    );
    let config = LocalWorkspaceConfig {
        workspace_root,
        artifact_root: temporary.path().join("artifacts"),
        repository_root: temporary.path().to_path_buf(),
        git_binary: Path::new("/usr/bin/git").to_path_buf(),
        limits: WorkspaceLimits::default(),
    };
    fs::create_dir_all(&config.workspace_root).expect("workspace root");
    let repository_id = Uuid::new_v4();
    let request = RuntimeGitWorkspaceRequest {
        repository_id,
        target_repository_id: repository_id,
        target_ref: String::from("refs/heads/main"),
        target_commit: commit.clone(),
        git_operations: vec![String::from("fetch")],
        ref_globs: vec![String::from("refs/heads/main")],
    };
    let (tree, _) = materialize_runtime_git(&config, &repository, &request, &staging, &active)
        .expect("materialize runtime Git worktree");
    assert_eq!(git_output_test(&active, &["rev-parse", "HEAD"]), commit);
    assert_eq!(git_output_test(&active, &["rev-parse", "HEAD^"]), parent);
    assert_eq!(
        git_output_test(&active, &["rev-parse", concat!("HEAD^", "{tree}")]),
        tree
    );
    assert_eq!(
        reachable_object_ids(&source, &commit),
        reachable_object_ids(&active, "refs/heads/main"),
        "runtime Git checkout must contain every object reachable from the target commit"
    );
    let source_object_inodes = regular_file_inodes(&repository.join("objects"));
    let checkout_object_inodes = regular_file_inodes(&active.join(".git/objects"));
    assert!(
        !source_object_inodes.is_empty(),
        "source Git object storage scan must find regular files"
    );
    assert!(
        !checkout_object_inodes.is_empty(),
        "runtime Git object storage scan must find regular files"
    );
    assert!(
        source_object_inodes.is_disjoint(&checkout_object_inodes),
        "runtime Git checkout must not share hard-linked object files with its source"
    );
    let config_text = fs::read_to_string(active.join(".git/config")).expect("Git config");
    assert!(config_text.contains("[remote \"origin\"]"));
    assert_eq!(
        git_output_test(&active, &["remote", "get-url", "origin"]),
        format!("http://127.0.0.1:19100/{repository_id}")
    );
    assert!(!active.join(".git/objects/info/alternates").exists());
    assert!(active.join("session.txt").is_file());
    assert!(!active.join("source").exists());
    assert!(!active.join("work").exists());
}

fn reachable_object_ids(directory: &Path, revision: &str) -> BTreeSet<String> {
    git_output_test(directory, &["rev-list", "--objects", revision])
        .lines()
        .filter_map(|line| line.split_ascii_whitespace().next())
        .map(str::to_owned)
        .collect()
}

fn regular_file_inodes(root: &Path) -> BTreeSet<(u64, u64)> {
    let mut inodes = BTreeSet::new();
    collect_regular_file_inodes(root, &mut inodes);
    inodes
}

fn collect_regular_file_inodes(root: &Path, inodes: &mut BTreeSet<(u64, u64)>) {
    for entry in fs::read_dir(root).expect("read Git object storage") {
        let entry = entry.expect("read Git object storage entry");
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).expect("stat Git object storage entry");
        if metadata.is_dir() {
            collect_regular_file_inodes(&path, inodes);
        } else if metadata.is_file() {
            inodes.insert((metadata.dev(), metadata.ino()));
        }
    }
}

fn run_git(directory: &Path, arguments: &[&str]) {
    let output = Command::new("/usr/bin/git")
        .current_dir(directory)
        .args(arguments)
        .output()
        .expect("run Git");
    assert!(
        output.status.success(),
        "Git {:?}: {}",
        arguments,
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_output_test(directory: &Path, arguments: &[&str]) -> String {
    let output = Command::new("/usr/bin/git")
        .current_dir(directory)
        .args(arguments)
        .output()
        .expect("run Git");
    assert!(output.status.success(), "Git {arguments:?}");
    String::from_utf8(output.stdout)
        .expect("Git UTF-8")
        .trim()
        .to_owned()
}
