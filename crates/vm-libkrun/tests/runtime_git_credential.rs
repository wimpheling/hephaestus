//! Exercises the installed helper through Git's real credential protocol.

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::Path,
    process::{Command, Stdio},
};

#[test]
fn git_credential_fill_uses_the_bound_runtime_route() {
    let directory = tempfile::tempdir().expect("temporary authority directory");
    let authority = directory.path().join("authority.json");
    fs::write(
        &authority,
        br#"{"runtime_git_credential":"abababababababababababababababababababababababababababababababab"}"#,
    )
    .expect("write authority");
    fs::set_permissions(&authority, fs::Permissions::from_mode(0o400)).expect("protect authority");

    let repository = "11111111-1111-4111-8111-111111111111";
    let output = run_fill(
        &authority,
        repository,
        &format!("http://127.0.0.1:19100/{repository}"),
    );
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8(output.stdout).expect("credential response is UTF-8");
    assert!(stdout.contains("username=heph-runtime\n"));
    let password = stdout
        .lines()
        .find_map(|line| line.strip_prefix("password="))
        .expect("password field");
    assert!(password.starts_with("heph_git_v1_"));
    assert_eq!(password.len(), "heph_git_v1_".len() + 43);
}

#[test]
fn git_credential_fill_rejects_a_different_repository_route() {
    let directory = tempfile::tempdir().expect("temporary authority directory");
    let authority = directory.path().join("authority.json");
    fs::write(
        &authority,
        br#"{"runtime_git_credential":"abababababababababababababababababababababababababababababababab"}"#,
    )
    .expect("write authority");
    fs::set_permissions(&authority, fs::Permissions::from_mode(0o400)).expect("protect authority");

    let output = run_fill(
        &authority,
        "11111111-1111-4111-8111-111111111111",
        "http://127.0.0.1:19100/22222222-2222-4222-8222-222222222222",
    );
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("helper stderr is UTF-8");
    assert_eq!(
        stderr.lines().next(),
        Some("heph_git_credential_error=target")
    );
    assert!(!stderr.contains("abababab"));
}

#[test]
fn git_credential_fill_reports_fixed_host_failure_code() {
    let directory = tempfile::tempdir().expect("temporary authority directory");
    let authority = directory.path().join("authority.json");
    fs::write(
        &authority,
        br#"{"runtime_git_credential":"abababababababababababababababababababababababababababababababab"}"#,
    )
    .expect("write authority");
    fs::set_permissions(&authority, fs::Permissions::from_mode(0o400)).expect("protect authority");

    let output = run_fill_with(
        &authority,
        "127.0.0.1:19101",
        "11111111-1111-4111-8111-111111111111",
        "http://127.0.0.1:19100/11111111-1111-4111-8111-111111111111",
    );
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("helper stderr is UTF-8");
    assert_eq!(
        stderr.lines().next(),
        Some("heph_git_credential_error=target")
    );
    assert!(!stderr.contains("abababab"));
}

#[test]
fn git_credential_fill_reports_fixed_protection_failure_code() {
    let directory = tempfile::tempdir().expect("temporary authority directory");
    let authority = directory.path().join("authority.json");
    fs::write(
        &authority,
        br#"{"runtime_git_credential":"abababababababababababababababababababababababababababababababab"}"#,
    )
    .expect("write authority");
    fs::set_permissions(&authority, fs::Permissions::from_mode(0o644))
        .expect("make authority too broad");

    let output = run_fill(
        &authority,
        "11111111-1111-4111-8111-111111111111",
        "http://127.0.0.1:19100/11111111-1111-4111-8111-111111111111",
    );
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("helper stderr is UTF-8");
    assert_eq!(
        stderr.lines().next(),
        Some("heph_git_credential_error=authority_protection")
    );
    assert!(!stderr.contains("abababab"));
}

fn run_fill(authority: &Path, repository: &str, url: &str) -> std::process::Output {
    run_fill_with(authority, "127.0.0.1:19100", repository, url)
}

fn run_fill_with(
    authority: &Path,
    expected_host: &str,
    repository: &str,
    url: &str,
) -> std::process::Output {
    let mut git = Command::new("git");
    git.args(["credential", "fill"])
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("HEPH_RUNTIME_AUTHORITY_PATH", authority)
        .env("HEPH_RUNTIME_GIT_HOST", expected_host)
        .env("HEPH_RUNTIME_GIT_PATH", repository)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_CONFIG_COUNT", "2")
        .env("GIT_CONFIG_KEY_0", "credential.helper")
        .env(
            "GIT_CONFIG_VALUE_0",
            env!("CARGO_BIN_EXE_heph-git-credential"),
        )
        .env("GIT_CONFIG_KEY_1", "credential.useHttpPath")
        .env("GIT_CONFIG_VALUE_1", "true")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = git.spawn().expect("spawn git credential fill");
    {
        use std::io::Write;
        child
            .stdin
            .take()
            .expect("credential stdin")
            .write_all(format!("url={url}\n\n").as_bytes())
            .expect("write credential request");
    }
    child
        .wait_with_output()
        .expect("wait for git credential fill")
}
