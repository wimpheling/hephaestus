use super::{UiManifestEntryKind, UiManifestStatus, inspect_repository_ui};
use std::{path::Path, process::Command};

const STATIC_UI: &str = r#"
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

[uis.content]
kind = "static"
entrypoint = "index.html"

[[uis.content.files]]
route = "index.html"
artifact = "dist/index.html"
media_type = "text/html"
"#;

const PRIVATE_GATEWAY: &str = r#"
version = 1

[[gateways]]
name = "private-ui"
agent_name = "ui-agent"
handler_contract = "http.service.v1"
exposure = "heph_authenticated"

[gateways.service]
loopback_port = 8080
readiness_path = "/ready"
health_path = "/health"

[[gateways.routes]]
path = "/ui"
methods = ["GET"]
"#;

const MANAGED_UI: &str = r#"
version = 1

[[uis]]
key = "service"
scope = "project"
label = "Service"
icon = "app"
presentation = "iframe"
route_base = "service"
ui_kit_version = 1
cache = "no_store"

[uis.content]
kind = "managed_service"
gateway_name = "private-ui"
route = "/ui"
entrypoint = "index.html"
"#;

#[test]
fn valid_static_source_does_not_read_an_unused_gateway_manifest() {
    let repository = fixture(&[("heph.ui.toml", STATIC_UI.as_bytes())]);
    let result = inspect(&repository);
    let result = result.expect("UI source").expect("inspection");
    assert_eq!(result.status, UiManifestStatus::Valid);
    assert!(!result.requires_gateways);
    assert_eq!(result.entry_kind, UiManifestEntryKind::Regular);
    assert!(result.actual_size.is_some());
    assert!(result.source_hash.is_some());
    assert!(result.normalized_hash.is_some());
    assert!(result.gateway_object_id.is_none());
    assert!(result.gateway_actual_size.is_none());
    assert!(result.gateway_source_hash.is_none());
    assert!(result.gateway_normalized_hash.is_none());
    assert!(result.gateway_config.is_none());
    assert!(result.diagnostics.is_empty());
}

#[test]
fn valid_managed_service_requires_private_gateway() {
    let repository = fixture(&[
        ("heph.ui.toml", MANAGED_UI.as_bytes()),
        ("heph.gateways.toml", PRIVATE_GATEWAY.as_bytes()),
    ]);
    let result = inspect(&repository)
        .expect("UI source")
        .expect("inspection");
    assert_eq!(result.status, UiManifestStatus::Valid);
    assert!(result.requires_gateways);
    assert!(result.actual_size.is_some());
    assert!(result.source_hash.is_some());
    assert!(result.normalized_hash.is_some());
    assert!(result.gateway_object_id.is_some());
    assert!(result.gateway_actual_size.is_some());
    assert!(result.gateway_source_hash.is_some());
    assert!(result.gateway_normalized_hash.is_some());
    assert!(result.gateway_config.is_some());
    assert!(result.config.is_some());
}

#[test]
fn missing_ui_is_legacy_none() {
    let repository = fixture(&[]);
    assert!(inspect(&repository).expect("inspection").is_none());
}

#[test]
fn invalid_toml_is_redacted() {
    let marker = ["opaque", "ui", "value"].concat();
    let source = format!("version = 1\nunknown_field = \"{marker}\"\n");
    let repository = fixture(&[("heph.ui.toml", source.as_bytes())]);
    let result = inspect(&repository)
        .expect("UI source")
        .expect("inspection");
    assert_eq!(result.status, UiManifestStatus::Invalid);
    assert!(!result.requires_gateways);
    assert!(result.source_hash.is_some());
    assert!(result.normalized_hash.is_none());
    assert!(!format!("{result:?}").contains(&marker));
}

#[test]
fn symlink_tree_and_gitlink_are_invalid_without_object_lookup() {
    for mode in ["symlink", "tree", "gitlink"] {
        let repository = fixture_with_special_entry(mode);
        let result = inspect(&repository).expect("inspection").expect("entry");
        assert_eq!(result.status, UiManifestStatus::Invalid);
        assert!(!result.requires_gateways);
        assert_ne!(result.entry_kind, UiManifestEntryKind::Regular);
        assert!(result.actual_size.is_none());
        assert!(result.gateway_object_id.is_none());
    }
}

#[test]
fn exact_ui_cap_is_accepted_and_oversize_is_rejected_before_blob_read() {
    let exact = format!(
        "{STATIC_UI}\n#{}",
        "x".repeat(262_144 - STATIC_UI.len() - 2)
    );
    let repository = fixture(&[("heph.ui.toml", exact.as_bytes())]);
    let result = inspect(&repository)
        .expect("UI source")
        .expect("inspection");
    assert_eq!(result.status, UiManifestStatus::Valid);
    assert_eq!(result.actual_size, Some(262_144));

    let oversized = format!(
        "{STATIC_UI}\n#{}",
        "x".repeat(262_145 - STATIC_UI.len() - 2)
    );
    let repository = fixture(&[("heph.ui.toml", oversized.as_bytes())]);
    let result = inspect(&repository)
        .expect("UI source")
        .expect("inspection");
    assert_eq!(result.status, UiManifestStatus::Invalid);
    assert!(!result.requires_gateways);
    assert_eq!(result.entry_kind, UiManifestEntryKind::Regular);
    assert!(result.actual_size.is_some());
    assert!(result.source_hash.is_none());
    assert!(result.normalized_hash.is_none());
}

#[test]
fn required_gateway_missing_or_oversize_is_invalid() {
    let repository = fixture(&[("heph.ui.toml", MANAGED_UI.as_bytes())]);
    let result = inspect(&repository)
        .expect("UI source")
        .expect("inspection");
    assert_eq!(result.status, UiManifestStatus::Invalid);
    assert!(result.requires_gateways);
    assert!(result.gateway_object_id.is_none());
    assert!(result.gateway_actual_size.is_none());
    assert!(result.gateway_source_hash.is_none());
    assert!(result.gateway_normalized_hash.is_none());

    let oversized = format!("version = 1\n#{}", "x".repeat(1_048_577));
    let repository = fixture(&[
        ("heph.ui.toml", MANAGED_UI.as_bytes()),
        ("heph.gateways.toml", oversized.as_bytes()),
    ]);
    let result = inspect(&repository)
        .expect("UI source")
        .expect("inspection");
    assert_eq!(result.status, UiManifestStatus::Invalid);
    assert!(result.requires_gateways);
    assert!(result.gateway_object_id.is_some());
    assert!(
        result
            .gateway_actual_size
            .is_some_and(|size| size > 1_048_576)
    );
    assert!(result.gateway_source_hash.is_none());
    assert!(result.gateway_normalized_hash.is_none());
    assert!(result.gateway_config.is_none());

    let marker = ["opaque", "gateway", "value"].concat();
    let malformed = format!("version = 1\nunknown_field = \"{marker}\"\n");
    let repository = fixture(&[
        ("heph.ui.toml", MANAGED_UI.as_bytes()),
        ("heph.gateways.toml", malformed.as_bytes()),
    ]);
    let result = inspect(&repository)
        .expect("UI source")
        .expect("inspection");
    assert_eq!(result.status, UiManifestStatus::Invalid);
    assert!(result.requires_gateways);
    assert!(result.gateway_object_id.is_some());
    assert!(result.gateway_actual_size.is_some());
    assert!(result.gateway_source_hash.is_some());
    assert!(result.gateway_normalized_hash.is_none());
    assert!(result.gateway_config.is_none());
    assert!(!format!("{result:?}").contains(&marker));
}

#[test]
fn invalid_gateway_reference_retains_observation_but_not_authoritative_config() {
    let gateway = PRIVATE_GATEWAY.replace("path = \"/ui\"", "path = \"/other\"");
    let repository = fixture(&[
        ("heph.ui.toml", MANAGED_UI.as_bytes()),
        ("heph.gateways.toml", gateway.as_bytes()),
    ]);
    let result = inspect(&repository)
        .expect("UI source")
        .expect("inspection");
    assert_eq!(result.status, UiManifestStatus::Invalid);
    assert!(result.requires_gateways);
    assert!(result.gateway_object_id.is_some());
    assert!(result.gateway_actual_size.is_some());
    assert!(result.gateway_source_hash.is_some());
    assert!(result.gateway_normalized_hash.is_none());
    assert!(result.gateway_config.is_none());
}

#[test]
fn corrupt_regular_object_is_fatal() {
    let repository = fixture(&[("heph.ui.toml", STATIC_UI.as_bytes())]);
    let object = git_output(&repository, &["hash-object", "heph.ui.toml"]);
    let object_path = repository
        .join(".git/objects")
        .join(&object[..2])
        .join(&object[2..]);
    std::fs::remove_file(object_path).expect("remove blob object");
    assert!(inspect(&repository).is_err());
}

fn inspect(
    repository: &Path,
) -> Result<Option<super::UiManifestInspection>, forge_service::ForgeRepositoryError> {
    let git_repository = gix::open(repository).expect("open fixture");
    let commit = git_output(repository, &["rev-parse", "HEAD"]);
    let commit = gix::ObjectId::from_hex(commit.as_bytes()).expect("commit ID");
    let tree = git_repository
        .find_commit(commit)
        .expect("commit")
        .tree()
        .expect("tree");
    inspect_repository_ui(&git_repository, &tree)
}

fn fixture(files: &[(&str, &[u8])]) -> std::path::PathBuf {
    let directory = tempfile::tempdir().expect("fixture directory");
    let path = directory.keep();
    git(&path, &["init"]);
    git(&path, &["config", "user.name", "UI fixture"]);
    git(&path, &["config", "user.email", "ui@example.invalid"]);
    if files.is_empty() {
        std::fs::write(path.join(".keep"), b"fixture").expect("placeholder");
    }
    for (name, contents) in files {
        let file = path.join(name);
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent).expect("parent");
        }
        std::fs::write(file, contents).expect("write fixture");
    }
    git(&path, &["add", "."]);
    git(&path, &["commit", "-m", "fixture"]);
    path
}

fn fixture_with_special_entry(mode: &str) -> std::path::PathBuf {
    let path = fixture(&[]);
    match mode {
        "symlink" => {
            std::os::unix::fs::symlink("target", path.join("heph.ui.toml")).expect("symlink");
            git(&path, &["add", "heph.ui.toml"]);
        }
        "tree" => {
            std::fs::create_dir(path.join("heph.ui.toml")).expect("tree");
            std::fs::write(path.join("heph.ui.toml/child"), b"x").expect("tree child");
            git(&path, &["add", "heph.ui.toml"]);
        }
        "gitlink" => {
            git(
                &path,
                &[
                    "update-index",
                    "--add",
                    "--cacheinfo",
                    "160000,1111111111111111111111111111111111111111,heph.ui.toml",
                ],
            );
        }
        _ => panic!("unknown fixture mode"),
    }
    git(&path, &["commit", "-m", "special entry"]);
    path
}

fn git(directory: &Path, arguments: &[&str]) {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(directory)
        .output()
        .expect("run Git");
    assert!(
        output.status.success(),
        "git {arguments:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_output(directory: &Path, arguments: &[&str]) -> String {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(directory)
        .output()
        .expect("run Git");
    assert!(output.status.success(), "git {arguments:?} failed");
    String::from_utf8(output.stdout)
        .expect("UTF-8 Git output")
        .trim()
        .to_owned()
}
