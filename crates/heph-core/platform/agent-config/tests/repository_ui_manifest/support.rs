//! Shared repository UI manifest fixture and diagnostic assertion.

use agent_config::ui::ParsedRepositoryUis;

/// Valid manifest covering static and managed-service UI declarations.
pub const VALID_STATIC: &str = r#"version = 1

[[uis]]
key = "assistant"
scope = "project"
label = "Assistant"
icon = "chat"
presentation = "iframe"
route_base = "assistant"
ui_kit_version = 1
cache = "no_store"

[[uis.apis]]
key = "release-read"
gateway_name = "release-api"
method = "GET"
route = "/releases"

[uis.content]
kind = "static"
entrypoint = "index.html"

[[uis.content.files]]
route = "app.js"
artifact = "dist/app.js"
media_type = "text/javascript"

[[uis.content.files]]
route = "index.html"
artifact = "dist/index.html"
media_type = "text/html"

[[uis]]
key = "ops"
scope = "global"
label = "Operations"
icon = "chart"
presentation = "full_page"
route_base = "ops"
ui_kit_version = 1
cache = "no_store"

[uis.content]
kind = "managed_service"
gateway_name = "ops-service"
route = "/ops"
entrypoint = "index.html"
"#;

/// Asserts that parsing failed with one expected diagnostic code.
pub fn assert_code(parsed: &ParsedRepositoryUis, expected: &str) {
    assert!(parsed.config.is_none(), "unexpected valid config");
    assert!(
        parsed
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == expected),
        "expected {expected}, got {:?}",
        parsed.diagnostics
    );
}
