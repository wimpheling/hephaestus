use crate::{REUSABLE_RELEASE_VERSION, parse};

#[test]
fn parses_reusable_release_contract_and_symbolic_slots() {
    let source = format!(
        r#"
version = {REUSABLE_RELEASE_VERSION}

[agent]
name = "Reviewer"
key = "reviewer"

[build]
command = "/bin/build"
arguments = ["--release"]
working_directory = "/workspace/source"
image = {{ key = "python-ubuntu" }}
triggers = ["refs/heads/main"]

[build.resources]
vcpus = 2
memory_mib = 1024

[build.network]
profile = "disabled"

[[build.artifacts]]
path = "bin/reviewer"
kind = "executable"
media_type = "application/octet-stream"

[guest]
image = {{ key = "python-ubuntu" }}
command = "bin/reviewer"
arguments = ["--json"]
working_directory = "bin"

[resources]
vcpus = 4
memory_mib = 2048

[workspace]
mount = true
path = "/workspace/repo"
read_only = true

[state_volume]
enabled = true

[network]
profile = "broker_only"

[triggers]
push = false
refs = []

[[parameters]]
name = "severity"
type = "enum"
values = ["warning", "error"]
required = false
default = "warning"

[[parameters]]
name = "max_findings"
type = "integer"
minimum = 1
maximum = 100
required = true

[[secret_slots]]
key = "model"
purpose = "Call the configured model provider"
required = true
delivery_modes = ["brokered"]
phases = ["normal", "update"]
destinations = ["api.model.example"]

[update_hook]
command = "bin/migrate"
arguments = ["--transactional"]
timeout_seconds = 300

[update_hook.resources]
vcpus = 1
memory_mib = 512
"#
    );
    let parsed = parse(source.as_bytes());
    assert!(
        parsed.diagnostics.is_empty(),
        "unexpected diagnostics: {:?}",
        parsed.diagnostics
    );
    let config = parsed.config.expect("version 2 should parse");
    assert_eq!(config.version, REUSABLE_RELEASE_VERSION);
    assert_eq!(config.agent.key.as_deref(), Some("reviewer"));
    assert_eq!(config.parameters.len(), 2);
    assert_eq!(config.secret_slots.len(), 1);
    assert!(config.capability_slots.is_empty());
    assert!(config.update_hook.is_some());

    let second = parse(source.as_bytes());
    assert_eq!(parsed.normalized_hash, second.normalized_hash);
}

#[test]
fn rejects_tenant_secret_material_and_unsafe_release_paths() {
    let source = r#"
version = 2
[agent]
name = "Reviewer"
key = "reviewer"
[build]
command = "/bin/build"
working_directory = "/source"
image = { key = "ubuntu-native" }
[build.resources]
vcpus = 1
memory_mib = 512
[build.network]
profile = "disabled"
[[build.artifacts]]
path = "../escape"
kind = "file"
[guest]
image = { key = "ubuntu-native" }
command = "/host/path"
working_directory = "../source"
[resources]
vcpus = 1
memory_mib = 512
[workspace]
mount = true
path = "/workspace/repo"
[state_volume]
enabled = false
[network]
profile = "disabled"
[triggers]
push = false
[[secret_slots]]
key = "model"
purpose = "model"
delivery_modes = ["brokered"]
phases = ["normal"]
secret_id = "f774d581-c89e-4420-9712-24cc642d2a9a"
plaintext = "must-never-be-accepted"
"#;
    let parsed = parse(source.as_bytes());
    assert!(parsed.config.is_none());
    assert_eq!(parsed.diagnostics[0].code, "invalid_toml");
    assert!(
        !format!("{:?}", parsed.diagnostics).contains("must-never-be-accepted"),
        "parser diagnostics must not echo rejected plaintext"
    );
}
