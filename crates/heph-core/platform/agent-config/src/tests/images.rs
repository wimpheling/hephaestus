use super::support::VALID;
use crate::{parse, parse_repository_oci_images};

#[test]
fn parses_image_selection_for_both_execution_contexts() {
    let source = VALID.replace("ubuntu-native", "typescript-tools");
    let parsed = parse(source.as_bytes());
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let config = parsed.config.expect("valid image selection");
    assert_eq!(
        config
            .build
            .as_ref()
            .and_then(|build| build.image.key.as_deref()),
        Some("typescript-tools"),
    );
    assert_eq!(config.guest.image.key.as_deref(), Some("typescript-tools"));
}

#[test]
fn accepts_a_project_image_for_an_isolated_build_only() {
    let source = VALID.replacen(
        "image = { key = \"ubuntu-native\" }",
        "image = { project_image = \"cooking-blog-hugo\" }",
        1,
    );
    let parsed = parse(source.as_bytes());
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    assert_eq!(
        parsed
            .config
            .and_then(|config| config.build)
            .and_then(|build| build.image.project_image),
        Some(String::from("cooking-blog-hugo"))
    );
}

#[test]
fn rejects_a_project_image_for_a_guest() {
    let source = VALID.replacen(
        "image = { key = \"ubuntu-native\" }",
        "image = { project_image = \"cooking-blog-hugo\" }",
        2,
    );
    let parsed = parse(source.as_bytes());
    assert!(parsed.config.is_none());
    assert!(
        parsed
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "project_image_not_permitted")
    );
}

#[test]
fn rejects_repository_supplied_immutable_image_references() {
    let source = VALID.replace(
        "image = { key = \"ubuntu-native\" }",
        "image = { key = \"ubuntu-native\", reference = \"registry.example/ubuntu@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\" }",
    );
    let parsed = parse(source.as_bytes());
    assert!(parsed.config.is_none());
    assert_eq!(parsed.diagnostics[0].code, "invalid_toml");
}

#[test]
fn rejects_unsafe_repository_oci_image_paths_and_duplicate_keys() {
    let manifest = r#"
version = 1

[[images]]
key = "typescript-tools"
display_name = "TypeScript tools"
[images.build]
dockerfile = "../Dockerfile"
context = "."
base = { key = "typescript-node-ubuntu" }

[[images]]
key = "typescript-tools"
display_name = "Duplicate"
[images.build]
dockerfile = "Dockerfile"
context = "../../host"
base = { key = "node:latest" }
"#;
    let parsed = parse_repository_oci_images(manifest.as_bytes());
    assert!(parsed.config.is_none());
    let codes = parsed
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        codes,
        [
            "invalid_repository_oci_image_dockerfile",
            "duplicate_repository_oci_image_key",
            "invalid_repository_oci_image_context",
            "invalid_repository_oci_image_base",
        ]
    );
}
