//! Parser contract tests for the repository-owned release UI manifest.

use agent_config::ui::{
    MAX_REPOSITORY_UIS, MAX_REPOSITORY_UIS_BYTES, MAX_UI_APIS, MAX_UI_FILES,
    REPOSITORY_UIS_VERSION, UI_KIT_VERSION,
};
use agent_config::{parse_repository_uis, ui::ParsedRepositoryUis};
use sha2::{Digest, Sha256};
use std::fmt::Write as _;

const VALID_STATIC: &str = r#"
version = 1

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

#[test]
fn parses_static_and_managed_service_declarations() {
    let parsed = parse_repository_uis(VALID_STATIC.as_bytes());
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let config = parsed.config.expect("valid UI manifest");
    assert_eq!(config.version, REPOSITORY_UIS_VERSION);
    assert_eq!(config.uis.len(), 2);
}

#[test]
fn normalizes_ui_api_file_and_ui_order_into_canonical_json_hash() {
    let first = VALID_STATIC;
    let second = r#"
version = 1

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
route = "index.html"
artifact = "dist/index.html"
media_type = "text/html"

[[uis.content.files]]
route = "app.js"
artifact = "dist/app.js"
media_type = "text/javascript"
"#;
    let left = parse_repository_uis(first.as_bytes());
    let right = parse_repository_uis(second.as_bytes());
    assert!(left.diagnostics.is_empty(), "{:?}", left.diagnostics);
    assert!(right.diagnostics.is_empty(), "{:?}", right.diagnostics);
    assert_ne!(left.hash, right.hash);
    assert_eq!(left.normalized_hash, right.normalized_hash);
    let normalized = serde_json::to_vec(left.config.as_ref().expect("normalized config")).unwrap();
    let digest = Sha256::digest(normalized);
    let mut expected = String::with_capacity(64);
    for byte in digest {
        write!(&mut expected, "{byte:02x}").expect("writing to a String cannot fail");
    }
    assert_eq!(left.normalized_hash.as_ref().unwrap().as_str(), expected);
    assert_eq!(
        left.config.as_ref().unwrap().uis[0].key.as_str(),
        "assistant"
    );
    assert_eq!(left.config.as_ref().unwrap().uis[1].key.as_str(), "ops");
}

#[test]
fn rejects_unknown_fields_and_does_not_echo_secret_values() {
    let parsed = parse_repository_uis(
        br#"
version = 1
secret_token = "sentinel-ui-secret-value"
"#,
    );
    assert!(parsed.config.is_none());
    assert_eq!(parsed.diagnostics[0].code, "invalid_toml");
    assert!(!format!("{:?}", parsed.diagnostics).contains("sentinel-ui-secret-value"));
}

#[test]
fn rejects_unsupported_versions_and_closed_values() {
    let version = format!("version = {}\n", REPOSITORY_UIS_VERSION + 1);
    let parsed = parse_repository_uis(version.as_bytes());
    assert_code(&parsed, "unsupported_repository_uis_version");

    let cache = VALID_STATIC.replace("cache = \"no_store\"", "cache = \"public\"");
    assert_code(&parse_repository_uis(cache.as_bytes()), "invalid_toml");

    let kit = VALID_STATIC.replace(
        "ui_kit_version = 1",
        &format!("ui_kit_version = {}", UI_KIT_VERSION + 1),
    );
    assert_code(
        &parse_repository_uis(kit.as_bytes()),
        "unsupported_repository_ui_kit_version",
    );
}

#[test]
fn rejects_duplicate_keys_labels_and_segment_overlapping_bases() {
    let duplicate_key = VALID_STATIC.replace("key = \"ops\"", "key = \"assistant\"");
    assert_code(
        &parse_repository_uis(duplicate_key.as_bytes()),
        "duplicate_repository_ui_key",
    );

    let duplicate_label = VALID_STATIC
        .replace("scope = \"global\"", "scope = \"project\"")
        .replace("label = \"Operations\"", "label = \"Assistant\"");
    assert_code(
        &parse_repository_uis(duplicate_label.as_bytes()),
        "duplicate_repository_ui_label",
    );

    let overlapping = VALID_STATIC
        .replace("scope = \"global\"", "scope = \"project\"")
        .replace("route_base = \"ops\"", "route_base = \"assistant/panel\"");
    assert_code(
        &parse_repository_uis(overlapping.as_bytes()),
        "conflicting_repository_ui_route_base",
    );

    let sibling = VALID_STATIC
        .replace("scope = \"global\"", "scope = \"project\"")
        .replace("route_base = \"ops\"", "route_base = \"assistant2\"");
    let parsed = parse_repository_uis(sibling.as_bytes());
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
}

#[test]
fn rejects_duplicate_api_keys_and_ambiguous_gateway_routes() {
    let duplicate_api = VALID_STATIC.replace(
        "[[uis.apis]]\nkey = \"release-read\"",
        "[[uis.apis]]\nkey = \"release-read\"\ngateway_name = \"release-api\"\nmethod = \"GET\"\nroute = \"/releases\"\n\n[[uis.apis]]\nkey = \"release-read\"",
    );
    assert_code(
        &parse_repository_uis(duplicate_api.as_bytes()),
        "duplicate_repository_ui_api_key",
    );

    let repeated_separator =
        VALID_STATIC.replace("route = \"/releases\"", "route = \"/release//read\"");
    assert_code(
        &parse_repository_uis(repeated_separator.as_bytes()),
        "invalid_repository_ui_api_route",
    );

    let service_backslash = VALID_STATIC.replace("route = \"/ops\"", "route = \"/ops\\\\service\"");
    assert_code(
        &parse_repository_uis(service_backslash.as_bytes()),
        "invalid_repository_ui_service_route",
    );
}

#[test]
fn rejects_static_route_duplicates_and_bad_entrypoints() {
    let duplicate_route = VALID_STATIC.replace(
        "route = \"app.js\"\nartifact = \"dist/app.js\"",
        "route = \"index.html\"\nartifact = \"dist/app.js\"",
    );
    assert_code(
        &parse_repository_uis(duplicate_route.as_bytes()),
        "duplicate_repository_ui_static_route",
    );

    let missing_entrypoint = VALID_STATIC.replace(
        "entrypoint = \"index.html\"",
        "entrypoint = \"missing.html\"",
    );
    assert_code(
        &parse_repository_uis(missing_entrypoint.as_bytes()),
        "invalid_repository_ui_entrypoint",
    );

    let non_html_entrypoint = VALID_STATIC
        .replace("entrypoint = \"index.html\"", "entrypoint = \"app.js\"")
        .replace(
            "route = \"index.html\"\nartifact = \"dist/index.html\"\nmedia_type = \"text/html\"",
            "route = \"index.html\"\nartifact = \"dist/index.html\"\nmedia_type = \"text/plain\"",
        );
    assert_code(
        &parse_repository_uis(non_html_entrypoint.as_bytes()),
        "invalid_repository_ui_entrypoint",
    );
}

#[test]
fn rejects_ui_route_and_artifact_traversal_or_url_forms() {
    let unsafe_route_base = VALID_STATIC.replace(
        "route_base = \"assistant\"",
        "route_base = \"https://example.invalid/ui\"",
    );
    assert_code(
        &parse_repository_uis(unsafe_route_base.as_bytes()),
        "invalid_toml",
    );

    let unsafe_artifact = VALID_STATIC.replace(
        "artifact = \"dist/index.html\"",
        "artifact = \"../secret.html\"",
    );
    assert_code(
        &parse_repository_uis(unsafe_artifact.as_bytes()),
        "invalid_toml",
    );
}

#[test]
fn enforces_manifest_ui_api_and_file_limits() {
    let mut uis = String::from("version = 1\n");
    for index in 0..=MAX_REPOSITORY_UIS {
        write!(
            &mut uis,
            "\n[[uis]]\nkey = \"ui-{index}\"\nscope = \"global\"\nlabel = \"UI {index}\"\nicon = \"app\"\npresentation = \"iframe\"\nroute_base = \"ui-{index}\"\nui_kit_version = 1\ncache = \"no_store\"\n\n[uis.content]\nkind = \"static\"\nentrypoint = \"index.html\"\n\n[[uis.content.files]]\nroute = \"index.html\"\nartifact = \"dist/{index}.html\"\nmedia_type = \"text/html\"\n"
        )
        .expect("writing to a String cannot fail");
    }
    assert_code(
        &parse_repository_uis(uis.as_bytes()),
        "too_many_repository_uis",
    );

    let mut apis = String::from(
        "version = 1\n\n[[uis]]\nkey = \"api-ui\"\nscope = \"global\"\nlabel = \"API UI\"\nicon = \"app\"\npresentation = \"iframe\"\nroute_base = \"api-ui\"\nui_kit_version = 1\ncache = \"no_store\"\n",
    );
    for index in 0..=MAX_UI_APIS {
        write!(
            &mut apis,
            "\n[[uis.apis]]\nkey = \"api-{index}\"\ngateway_name = \"release-api\"\nmethod = \"GET\"\nroute = \"/api-{index}\"\n"
        )
        .expect("writing to a String cannot fail");
    }
    apis.push_str(
        "\n[uis.content]\nkind = \"static\"\nentrypoint = \"index.html\"\n\n[[uis.content.files]]\nroute = \"index.html\"\nartifact = \"dist/index.html\"\nmedia_type = \"text/html\"\n",
    );
    assert_code(
        &parse_repository_uis(apis.as_bytes()),
        "too_many_repository_ui_apis",
    );

    let mut files = String::from(
        "version = 1\n\n[[uis]]\nkey = \"files-ui\"\nscope = \"global\"\nlabel = \"Files UI\"\nicon = \"app\"\npresentation = \"iframe\"\nroute_base = \"files-ui\"\nui_kit_version = 1\ncache = \"no_store\"\n\n[uis.content]\nkind = \"static\"\nentrypoint = \"index.html\"\n\n[[uis.content.files]]\nroute = \"index.html\"\nartifact = \"dist/index.html\"\nmedia_type = \"text/html\"\n",
    );
    for index in 0..MAX_UI_FILES {
        write!(
            &mut files,
            "\n[[uis.content.files]]\nroute = \"file-{index}.txt\"\nartifact = \"dist/file-{index}.txt\"\nmedia_type = \"text/plain\"\n"
        )
        .expect("writing to a String cannot fail");
    }
    assert_code(
        &parse_repository_uis(files.as_bytes()),
        "invalid_repository_ui_static_files",
    );
}

#[test]
fn rejects_oversized_source_before_toml_parsing() {
    let source = vec![b'x'; MAX_REPOSITORY_UIS_BYTES + 1];
    assert_code(&parse_repository_uis(&source), "repository_uis_too_large");
}

fn assert_code(parsed: &ParsedRepositoryUis, expected: &str) {
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
