//! Bare-Git source fixture helpers.

use std::{fs, path::Path};
use uuid::Uuid;

use super::CONFIG;

pub fn source_repository(root: &Path, repository_root: &Path) -> (Uuid, String) {
    let source = root.join("source");
    fs::create_dir(&source).expect("source");
    git(&source, &["init", "--initial-branch=main"]);
    git(&source, &["config", "user.name", "Build Test"]);
    git(&source, &["config", "user.email", "build@example.invalid"]);
    fs::write(source.join("agent.toml"), CONFIG).expect("agent config");
    fs::write(source.join("input.txt"), "exact source\n").expect("source input");
    git(&source, &["add", "."]);
    git(&source, &["commit", "-m", "build source"]);
    let commit = git_text(&source, &["rev-parse", "HEAD"]);
    let repository_id = Uuid::new_v4();
    let bare = repository_root.join(format!("{repository_id}.git"));
    git(
        root,
        &[
            "clone",
            "--bare",
            source.to_str().expect("source path"),
            bare.to_str().expect("bare path"),
        ],
    );
    (repository_id, commit)
}

fn git(directory: &Path, arguments: &[&str]) {
    let output = std::process::Command::new("/usr/bin/git")
        .args(arguments)
        .current_dir(directory)
        .output()
        .expect("run Git");
    assert!(
        output.status.success(),
        "Git failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_text(directory: &Path, arguments: &[&str]) -> String {
    let output = std::process::Command::new("/usr/bin/git")
        .args(arguments)
        .current_dir(directory)
        .output()
        .expect("run Git");
    assert!(output.status.success());
    String::from_utf8(output.stdout)
        .expect("Git UTF-8")
        .trim()
        .to_owned()
}
