//! Shared immutable build configuration fixture.

pub const CONFIG: &str = r#"
version = 2
[agent]
name = "isolated-builder"
key = "isolated-builder"
[build]
image = { key = "build" }
command = "/usr/bin/fake-build"
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
media_type = "application/x-hephaestus-test"
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
