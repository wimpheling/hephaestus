use super::*;

pub async fn postgres_pool() -> Option<PgPool> {
    let url = std::env::var("HEPHAESTUS_POSTGRES_TEST_URL").ok()?;
    Some(
        PgPoolOptions::new()
            .max_connections(5)
            .connect(&url)
            .await
            .expect("PostgreSQL test connection"),
    )
}

pub async fn git_exec_path() -> std::path::PathBuf {
    std::path::PathBuf::from(git_output(Path::new("."), &["--exec-path"]).await)
}

pub fn runtime_receive_hook_path() -> PathBuf {
    let hook = std::env::var_os("HEPHAESTUS_GIT_RECEIVE_HOOK").map_or_else(
        || {
            let test_executable =
                std::env::current_exe().expect("locate the active smart HTTP test executable");
            test_executable
                .parent()
                .and_then(Path::parent)
                .map(|profile| profile.join("pre-receive"))
                .expect("locate the active Cargo target profile")
        },
        PathBuf::from,
    );
    assert!(
        hook.is_absolute(),
        "runtime receive hook must be an absolute path: {}",
        hook.display()
    );
    assert_eq!(
        hook.file_name().and_then(|name| name.to_str()),
        Some("pre-receive"),
        "runtime receive hook must be named pre-receive: {}",
        hook.display()
    );
    assert!(
        hook.is_file(),
        "runtime receive hook is missing at {}; build it with `cargo build -p git-http --bin pre-receive` or set HEPHAESTUS_GIT_RECEIVE_HOOK",
        hook.display()
    );
    assert!(
        runtime_hook_is_executable(&hook),
        "runtime receive hook is not executable: {}",
        hook.display()
    );
    hook.canonicalize().unwrap_or(hook)
}

#[cfg(unix)]
pub fn runtime_hook_is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    std::fs::metadata(path)
        .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
pub fn runtime_hook_is_executable(path: &Path) -> bool {
    path.is_file()
}

pub async fn git(directory: &Path, arguments: &[&str]) {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(directory)
        .output()
        .await
        .expect("run git");
    assert!(
        output.status.success(),
        "git {arguments:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

pub async fn git_authenticated(directory: &Path, arguments: &[&str], authorization: &str) {
    let output = git_authenticated_result(directory, arguments, authorization).await;
    assert!(
        output.status.success(),
        "authenticated git {arguments:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

pub async fn git_authenticated_result(
    directory: &Path,
    arguments: &[&str],
    authorization: &str,
) -> std::process::Output {
    Command::new("git")
        .args(arguments)
        .current_dir(directory)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "http.extraHeader")
        .env(
            "GIT_CONFIG_VALUE_0",
            format!("Authorization: {authorization}"),
        )
        .output()
        .await
        .expect("run authenticated Git")
}

pub async fn git_output(directory: &Path, arguments: &[&str]) -> String {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(directory)
        .output()
        .await
        .expect("run git");
    assert!(
        output.status.success(),
        "git {arguments:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("UTF-8 Git output")
        .trim()
        .to_owned()
}

pub const fn valid_agent_config() -> &'static str {
    r#"
version = 1

[agent]
name = "reviewer"

[guest]
command = "/usr/bin/review"
arguments = ["--format=json"]
working_directory = "/workspace"

[resources]
vcpus = 2
memory_mib = 512

[root_image]
reference = "registry.example/agent@sha256:abc"

[workspace]
mount = true
path = "/workspace/repo"
read_only = true

[state_volume]
enabled = true

[network]
profile = "disabled"

[triggers]
push = true
refs = ["refs/heads/*"]
"#
}
