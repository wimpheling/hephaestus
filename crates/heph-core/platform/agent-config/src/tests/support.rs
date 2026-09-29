pub const VALID: &str = r#"
version = 2

[agent]
name = "reviewer"
key = "reviewer"

[build]
command = "/usr/bin/build"
working_directory = "/workspace/source"
image = { key = "ubuntu-native" }
triggers = ["refs/heads/main"]

[build.resources]
vcpus = 1
memory_mib = 512

[build.network]
profile = "disabled"

[[build.artifacts]]
path = "bin/review"
kind = "executable"

[guest]
image = { key = "ubuntu-native" }
command = "bin/review"
arguments = ["--format=json"]
working_directory = "bin"

[resources]
vcpus = 2
memory_mib = 512

[workspace]
mount = true
path = "/workspace/repo"
read_only = true

[state_volume]
enabled = true

[results]
declared_files = ["reports/review.json"]

[network]
profile = "disabled"

[triggers]
push = true
refs = ["refs/heads/*"]
"#;
