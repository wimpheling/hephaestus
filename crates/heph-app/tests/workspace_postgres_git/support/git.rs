use super::*;

pub fn image_key(repository_id: forge_domain::RepositoryId, context: &str) -> String {
    format!("workspace-{context}-{repository_id}")
}

pub fn agent_config(repository_id: forge_domain::RepositoryId) -> String {
    r#"
version = 2
[agent]
name = "workspace-agent"
key = "workspace-agent"
[build]
image = { key = "__BUILD_IMAGE_KEY__" }
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
image = { key = "__RUNTIME_IMAGE_KEY__" }
command = "bin/agent"
arguments = []
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
[results]
declared_files = ["reports/result.txt"]
[network]
profile = "disabled"
[triggers]
push = true
refs = ["refs/heads/main"]
"#
    .replace("__BUILD_IMAGE_KEY__", &image_key(repository_id, "build"))
    .replace(
        "__RUNTIME_IMAGE_KEY__",
        &image_key(repository_id, "runtime"),
    )
}

pub async fn git_binary() -> PathBuf {
    let output = Command::new("sh")
        .args(["-c", "command -v git"])
        .output()
        .await
        .expect("resolve Git executable");
    assert!(output.status.success());
    PathBuf::from(
        String::from_utf8(output.stdout)
            .expect("UTF-8 Git executable")
            .trim(),
    )
}

pub async fn git(directory: &Path, arguments: &[&str]) {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(directory)
        .output()
        .await
        .expect("run Git");
    assert!(
        output.status.success(),
        "git {arguments:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

pub async fn git_output(directory: &Path, arguments: &[&str]) -> String {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(directory)
        .output()
        .await
        .expect("run Git");
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

pub async fn git_output_bare(repository: &Path, arguments: &[&str]) -> String {
    let output = Command::new("git")
        .arg(format!("--git-dir={}", repository.display()))
        .args(arguments)
        .output()
        .await
        .expect("run bare Git");
    assert!(
        output.status.success(),
        "bare git {arguments:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("UTF-8 bare Git output")
        .trim()
        .to_owned()
}

pub async fn git_ref_exists(repository: &Path, git_ref: &str) -> bool {
    Command::new("git")
        .arg(format!("--git-dir={}", repository.display()))
        .args(["rev-parse", "--verify", git_ref])
        .output()
        .await
        .expect("inspect bare Git ref")
        .status
        .success()
}
