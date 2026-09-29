use super::*;
use crate::BuildInput;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::{fs, process::Command};

const CONFIG: &str = r#"
version = 2
[agent]
name = "builder"
key = "builder"
[build]
image = { key = "build" }
command = "/usr/bin/build"
working_directory = "/workspace/source"
triggers = ["refs/heads/main"]
[build.resources]
vcpus = 1
memory_mib = 128
[build.network]
profile = "disabled"
[[build.artifacts]]
path = "bin/agent"
kind = "executable"
[guest]
image = { key = "run" }
command = "bin/agent"
working_directory = "bin"
[resources]
vcpus = 1
memory_mib = 128
[workspace]
mount = true
path = "/workspace/repo"
read_only = true
[state_volume]
enabled = false
[network]
profile = "disabled"
[triggers]
push = false
refs = []
    "#;

#[test]
fn materializes_two_exact_commits_without_git_metadata() {
    let temporary = tempfile::tempdir().expect("temporary root");
    let repositories = temporary.path().join("repositories");
    let workspaces = temporary.path().join("builds");
    let source_repository = temporary.path().join("source");
    fs::create_dir(&repositories).expect("repository root");
    fs::create_dir_all(workspaces.join("active")).expect("workspace root");
    fs::create_dir(&source_repository).expect("source repository");
    git(&source_repository, &["init", "--initial-branch=main"]);
    git(&source_repository, &["config", "user.name", "Build Test"]);
    git(
        &source_repository,
        &["config", "user.email", "build@example.invalid"],
    );
    fs::write(source_repository.join("input.txt"), b"first\n").expect("first source");
    git(&source_repository, &["add", "input.txt"]);
    git(&source_repository, &["commit", "-m", "first"]);
    let first = git_text(&source_repository, &["rev-parse", "HEAD"]);
    fs::write(source_repository.join("input.txt"), b"second\n").expect("second source");
    git(&source_repository, &["commit", "-am", "second"]);
    let second = git_text(&source_repository, &["rev-parse", "HEAD"]);
    let repository_id = Uuid::new_v4();
    git(
        temporary.path(),
        &[
            "clone",
            "--bare",
            source_repository.to_str().expect("source path"),
            repositories
                .join(format!("{repository_id}.git"))
                .to_str()
                .expect("bare path"),
        ],
    );
    let config = BuildExecutorConfig {
        workspace_root: fs::canonicalize(&workspaces).expect("workspace path"),
        repository_root: fs::canonicalize(&repositories).expect("repository path"),
        git_binary: fs::canonicalize("/usr/bin/git").expect("Git binary"),
        image_filesystems: Arc::new(RwLock::new(BTreeMap::new())),
        timeout: Duration::from_secs(30),
    };
    let build = agent_config::parse(CONFIG.as_bytes())
        .config
        .expect("valid config")
        .build
        .expect("build config");
    for (index, (commit, expected)) in [
        (first, b"first\n".as_slice()),
        (second, b"second\n".as_slice()),
    ]
    .into_iter()
    .enumerate()
    {
        let id = BuildRequestId::new();
        let input = BuildInput {
            id,
            repository_id,
            source_commit: commit,
            source_ref: String::from("refs/heads/main"),
            build: build.clone(),
            image_reference: String::from(
                "build@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            ),
        };
        let active = config
            .workspace_root
            .join("active")
            .join(format!("{id}-{index}"));
        let prepared =
            prepare_workspace(&config, &input, &active).expect("materialize exact commit");
        assert_eq!(
            fs::read(prepared.source.join("input.txt")).expect("materialized source"),
            expected
        );
        assert!(!prepared.source.join(".git").exists());
        assert_eq!(
            fs::metadata(&prepared.source)
                .expect("source metadata")
                .permissions()
                .mode()
                & 0o222,
            0
        );
        assert!(
            fs::read_dir(&prepared.output)
                .expect("empty output")
                .next()
                .is_none()
        );
        cleanup_workspace(&prepared.root).expect("cleanup workspace");
    }
}

#[test]
fn sealing_rejects_guest_symlinks() {
    let temporary = tempfile::tempdir().expect("temporary root");
    let output = temporary.path().join("output");
    fs::create_dir(&output).expect("output");
    symlink("/etc/passwd", output.join("escape")).expect("guest symlink");
    assert!(matches!(
        seal_output(&output),
        Err(BuildExecutionError::UnsafeOutput)
    ));
}

fn git(directory: &Path, arguments: &[&str]) {
    let output = Command::new("/usr/bin/git")
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
    let output = Command::new("/usr/bin/git")
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
