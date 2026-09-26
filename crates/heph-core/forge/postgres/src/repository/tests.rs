use super::inspect::inspect_updates;
use forge_domain::{CommitSha, GitRef, RefUpdate};
use std::{path::Path, process::Command};

#[test]
fn discovers_repository_oci_image_inputs_from_the_exact_received_commit() {
    let temporary = tempfile::tempdir().expect("temporary repository");
    let bare = temporary.path().join("source.git");
    let worktree = temporary.path().join("source");
    git(
        temporary.path(),
        &["init", "--bare", bare.to_str().expect("UTF-8 path")],
    );
    git(
        temporary.path(),
        &["init", worktree.to_str().expect("UTF-8 path")],
    );
    git(&worktree, &["config", "user.name", "Hephaestus Test"]);
    git(
        &worktree,
        &["config", "user.email", "hephaestus@example.invalid"],
    );
    std::fs::create_dir_all(worktree.join("containers")).expect("containers directory");
    std::fs::write(
        worktree.join("heph.images.toml"),
        r#"
version = 1

[[images]]
key = "typescript-tools"
display_name = "TypeScript tools"

[images.build]
dockerfile = "containers/Dockerfile"
context = "."
base = { key = "typescript-node-ubuntu" }
"#,
    )
    .expect("manifest");
    std::fs::write(
        worktree.join("containers/Dockerfile"),
        "FROM heph-base AS build\nRUN echo ready\n",
    )
    .expect("Dockerfile");
    git(&worktree, &["add", "."]);
    git(&worktree, &["commit", "-m", "repository OCI image"]);
    let commit =
        CommitSha::parse(git_output(&worktree, &["rev-parse", "HEAD"])).expect("commit ID");
    git(
        &worktree,
        &[
            "push",
            bare.to_str().expect("UTF-8 path"),
            "HEAD:refs/heads/main",
        ],
    );

    let inspected = inspect_updates(
        &bare,
        &[RefUpdate {
            git_ref: GitRef::parse("refs/heads/main").expect("Git ref"),
            old_commit: None,
            new_commit: Some(commit),
        }],
    )
    .expect("inspect received commit");

    let images = inspected[0]
        .repository_oci_images
        .as_ref()
        .expect("repository OCI image manifest");
    assert_eq!(images.len(), 1);
    assert_eq!(images[0].key, "typescript-tools");
    assert_eq!(images[0].dockerfile_path, "containers/Dockerfile");
    assert_eq!(images[0].context_digest.len(), 71);
    assert!(images[0].context_digest.starts_with("sha256:"));
}

fn git(directory: &Path, arguments: &[&str]) {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(directory)
        .output()
        .expect("run Git");
    assert!(
        output.status.success(),
        "git {arguments:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_output(directory: &Path, arguments: &[&str]) -> String {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(directory)
        .output()
        .expect("run Git");
    assert!(output.status.success(), "git {arguments:?} failed");
    String::from_utf8(output.stdout)
        .expect("UTF-8 Git output")
        .trim()
        .to_owned()
}
