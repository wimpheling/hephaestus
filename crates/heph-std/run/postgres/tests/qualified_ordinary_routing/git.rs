use std::{path::Path, process::Command, sync::Arc};

use forge_domain::{CommitSha, RepositoryId};
use forge_storage::GitStorage;

pub async fn commit(root: &Path, repository: RepositoryId) -> (Arc<GitStorage>, CommitSha) {
    let storage = Arc::new(
        GitStorage::initialize(root.join("bare"))
            .await
            .expect("actual Git root"),
    );
    let bare = storage
        .create_bare(repository, "main")
        .await
        .expect("actual bare repository");
    let work = root.join("work");
    std::fs::create_dir(&work).expect("temporary worktree");
    git(&work, &["init", "--initial-branch=main"]);
    git(&work, &["config", "user.name", "Ordinary routing fixture"]);
    git(
        &work,
        &["config", "user.email", "ordinary-routing@example.invalid"],
    );
    std::fs::write(work.join("README"), b"Actual ordinary routing commit\n")
        .expect("commit content");
    git(&work, &["add", "README"]);
    git(
        &work,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "ordinary routing source",
        ],
    );
    let sha =
        CommitSha::parse(git(&work, &["rev-parse", "HEAD"]).trim()).expect("actual commit SHA");
    git(
        &work,
        &[
            "push",
            bare.to_str().expect("temporary UTF8 bare path"),
            "HEAD:refs/heads/main",
        ],
    );
    (storage, sha)
}

fn git(directory: &Path, arguments: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(directory)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .args(arguments)
        .output()
        .expect("local Git executable");
    assert!(
        output.status.success(),
        "Git {arguments:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("Git UTF8 stdout")
}
