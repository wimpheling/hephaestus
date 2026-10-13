//! Local real Git content; the production Forge API creates request evidence.

use forge_storage::GitStorage;
use heph_forge::{CommitSha, RepositoryId};
use std::{path::Path, process::Command};

pub(super) async fn commit(
    storage: &GitStorage,
    root: &Path,
    repository: RepositoryId,
) -> CommitSha {
    let bare = storage
        .create_bare(repository, "main")
        .await
        .expect("actual bare repository");
    let work = root.join("source");
    std::fs::create_dir(&work).expect("temporary source");
    git(&work, &["init", "--initial-branch=main"]);
    git(
        &work,
        &["config", "user.name", "Ordinary public139 fixture"],
    );
    git(
        &work,
        &["config", "user.email", "ordinary139@example.invalid"],
    );
    std::fs::write(work.join("README"), b"Real ordinary public139 source\n").expect("source bytes");
    git(&work, &["add", "README"]);
    git(
        &work,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "ordinary public139",
        ],
    );
    let commit =
        CommitSha::parse(git(&work, &["rev-parse", "HEAD"]).trim()).expect("actual commit");
    git(
        &work,
        &[
            "push",
            bare.to_str().expect("UTF8 fixture path"),
            "HEAD:refs/heads/main",
        ],
    );
    commit
}

fn git(root: &Path, arguments: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(root)
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
    String::from_utf8(output.stdout).expect("Git output")
}
