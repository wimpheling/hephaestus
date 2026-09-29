pub const CONFIG: &str = r#"
version = 2
[agent]
name = "Manual UI test agent"
key = "manual-ui"
[build]
image = { key = "manual-builder" }
command = "/bin/build"
working_directory = "/source"
triggers = ["refs/heads/main"]
[build.resources]
vcpus = 1
memory_mib = 256
[build.network]
profile = "disabled"
[[build.artifacts]]
path = "bin/app"
kind = "executable"
[guest]
image = { key = "manual-runtime" }
command = "bin/app"
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
[network]
profile = "disabled"
[triggers]
push = false
"#;
