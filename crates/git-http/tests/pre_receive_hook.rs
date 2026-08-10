//! Real Git coverage for quarantined runtime receive enforcement.

use git_capability_domain::{
    BranchRefPolicy, BranchUpdatePolicy, ChangedPathGlob, GitCapabilityScope,
    GitCapabilityScopeInput, GitOperation, RefGlob, RefMutationPermission, RefNamespacePolicy,
    RefUpdatePolicy, RepositoryId, TransferLimits,
};
use git_http::receive_policy::ResolvedRuntimeReceiveContext;
use std::{
    fs,
    os::unix::fs::symlink,
    path::Path,
    process::{Command, Output},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use tempfile::TempDir;
use uuid::Uuid;

#[test]
fn denied_quarantined_update_does_not_change_canonical_ref() {
    let fixture = TempDir::new().expect("temporary fixture");
    let repository_id = RepositoryId::new(Uuid::new_v4());
    let bare = fixture.path().join(format!("{repository_id}.git"));
    let work = fixture.path().join("work");
    git(
        fixture.path(),
        ["init", "--bare", bare.to_str().expect("bare path")],
    );
    git(fixture.path(), ["init", work.to_str().expect("work path")]);
    git(&work, ["config", "user.name", "Runtime test"]);
    git(&work, ["config", "user.email", "runtime@example.invalid"]);
    git(&work, ["checkout", "-b", "main"]);
    fs::create_dir_all(work.join("sessions")).expect("create allowed directory");
    fs::write(work.join("sessions/seed.json"), b"{}\n").expect("write allowed seed");
    fs::write(work.join("README.md"), b"human-owned\n").expect("write human seed");
    git(&work, ["add", "."]);
    git(&work, ["commit", "-m", "initial"]);
    git(&work, ["push", bare.to_str().expect("bare path"), "main"]);

    let hooks = fixture.path().join("runtime-hooks");
    fs::create_dir(&hooks).expect("create hook directory");
    symlink(env!("CARGO_BIN_EXE_pre-receive"), hooks.join("pre-receive"))
        .expect("install test hook");
    git(
        &bare,
        [
            "config",
            "core.hooksPath",
            hooks.to_str().expect("hooks path"),
        ],
    );

    fs::write(work.join("sessions/allowed.json"), b"{\"allowed\":true}\n")
        .expect("write allowed change");
    git(&work, ["add", "."]);
    git(&work, ["commit", "-m", "allowed runtime change"]);
    let allowed_commit = git_stdout(&work, ["rev-parse", "HEAD"]);
    let context = receive_context(repository_id);
    let allowed = runtime_push(&work, &bare, &context);
    assert!(
        allowed.status.success(),
        "allowed push failed: {}",
        String::from_utf8_lossy(&allowed.stderr)
    );
    assert_eq!(
        git_stdout(&bare, ["rev-parse", "refs/heads/main"]),
        allowed_commit
    );

    fs::create_dir_all(work.join("outside")).expect("create denied directory");
    fs::write(work.join("outside/denied.json"), b"{\"denied\":true}\n")
        .expect("write denied change");
    git(&work, ["add", "."]);
    git(&work, ["commit", "-m", "denied runtime change"]);
    let denied = runtime_push(&work, &bare, &context);
    assert!(!denied.status.success(), "out-of-scope push was accepted");
    assert!(
        String::from_utf8_lossy(&denied.stderr).contains("runtime receive denied"),
        "hook denial was not reported"
    );
    assert_eq!(
        git_stdout(&bare, ["rev-parse", "refs/heads/main"]),
        allowed_commit,
        "a denied receive changed the canonical ref"
    );
}

fn receive_context(repository_id: RepositoryId) -> ResolvedRuntimeReceiveContext {
    let now = now_unix_seconds();
    let scope = GitCapabilityScope::new(GitCapabilityScopeInput {
        repository_id,
        operations: vec![GitOperation::Receive],
        ref_globs: vec![RefGlob::parse("refs/heads/main").expect("ref glob")],
        changed_path_globs: vec![ChangedPathGlob::parse("sessions/**").expect("changed path glob")],
        update_policy: RefUpdatePolicy {
            branches: BranchRefPolicy {
                updates: BranchUpdatePolicy::FastForwardOnly,
                create: RefMutationPermission::Deny,
                delete: RefMutationPermission::Deny,
            },
            tags: RefNamespacePolicy::default(),
            other: RefNamespacePolicy::default(),
        },
        expires_at_unix_seconds: now + 3_600,
        transfer_limits: TransferLimits::new(16 * 1_024 * 1_024, 64 * 1_024 * 1_024, 100_000, 8)
            .expect("transfer limits"),
    })
    .expect("receive scope");
    ResolvedRuntimeReceiveContext::new(Arc::new(scope), "run-1", "snapshot-1", now)
        .expect("receive context")
}

fn runtime_push(work: &Path, bare: &Path, context: &ResolvedRuntimeReceiveContext) -> Output {
    let context_file = tempfile::NamedTempFile::new().expect("host context file");
    std::fs::write(
        context_file.path(),
        context.to_hook_json().expect("hook context"),
    )
    .expect("write host context");
    Command::new("git")
        .current_dir(work)
        .args(["push", bare.to_str().expect("bare path"), "main"])
        .env("HEPH_RUNTIME_RECEIVE_CONTEXT_FILE", context_file.path())
        .env(
            "HEPH_RUNTIME_RECEIVE_REPOSITORY",
            context.repository_id().to_string(),
        )
        .env("HEPH_RUNTIME_RECEIVE_REQUEST_BYTES", "16777216")
        .output()
        .expect("run runtime push")
}

fn git<const N: usize>(directory: &Path, arguments: [&str; N]) {
    let output = Command::new("git")
        .current_dir(directory)
        .args(arguments)
        .output()
        .expect("run Git");
    assert!(
        output.status.success(),
        "Git failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_stdout<const N: usize>(directory: &Path, arguments: [&str; N]) -> String {
    let output = Command::new("git")
        .current_dir(directory)
        .args(arguments)
        .output()
        .expect("run Git");
    assert!(output.status.success(), "Git command failed");
    String::from_utf8(output.stdout)
        .expect("Git output is UTF-8")
        .trim()
        .to_owned()
}

fn now_unix_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_secs()
        .try_into()
        .expect("Unix time fits i64")
}
