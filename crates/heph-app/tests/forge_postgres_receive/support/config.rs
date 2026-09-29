use super::*;

pub fn valid_config(repository_id: Uuid) -> String {
    r#"
version = 2
[agent]
name = "Reviewer"
key = "reviewer"
[build]
image = { key = "__BUILD_IMAGE_KEY__" }
command = "/bin/build"
working_directory = "/source"
triggers = ["refs/heads/main"]
[build.resources]
vcpus = 1
memory_mib = 512
[build.network]
profile = "disabled"
[[build.artifacts]]
path = "bin/reviewer"
kind = "executable"
[guest]
image = { key = "__RUNTIME_IMAGE_KEY__" }
command = "bin/reviewer"
arguments = []
working_directory = "bin"
[resources]
vcpus = 1
memory_mib = 256
[workspace]
mount = true
path = "/workspace/repo"
read_only = true
[state_volume]
enabled = true
[network]
profile = "disabled"
[triggers]
push = false
"#
    .replace(
        "__BUILD_IMAGE_KEY__",
        &fixture_image_key("build", repository_id),
    )
    .replace(
        "__RUNTIME_IMAGE_KEY__",
        &fixture_image_key("runtime", repository_id),
    )
}

pub fn fixture_image_key(kind: &str, repository_id: Uuid) -> String {
    format!("forge-{kind}-{}", repository_id.simple())
}

pub fn fixture_image_reference(kind: &str) -> String {
    let first = Uuid::new_v4().simple().to_string();
    let second = Uuid::new_v4().simple().to_string();
    format!("{kind}@sha256:{first}{second}")
}
