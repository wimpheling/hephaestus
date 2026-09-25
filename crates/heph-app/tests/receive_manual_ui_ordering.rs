//! Receive/manual UI capture ordering through two real application-role pools.

#![cfg(feature = "test-fixtures")]

const CONFIG_TEMPLATE: &str = r#"
version = 2
[agent]
name = "Receive ordering test agent"
key = "receive-ordering"
[build]
image = { key = "__BUILD_IMAGE__" }
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
image = { key = "__RUNTIME_IMAGE__" }
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

const VALID_UI: &str = r#"
version = 1
[[uis]]
key = "docs"
scope = "global"
label = "Docs"
icon = "book"
presentation = "iframe"
route_base = "docs"
ui_kit_version = 1
cache = "no_store"
[uis.content]
kind = "static"
entrypoint = "index.html"
[[uis.content.files]]
route = "index.html"
artifact = "dist/index.html"
media_type = "text/html"
"#;

const INVALID_UI: &str = "version = 1\nunknown_field = \"invalid\"\n";

#[path = "receive_manual_ui_ordering/fixture.rs"]
mod fixture;
#[path = "receive_manual_ui_ordering/ordering.rs"]
mod ordering;
