use super::{
    configuration::{challenge_matches, render_template},
    lifecycle::{start, stop},
};
use crate::context::DevContext;
use std::{env, path::PathBuf};
use tempfile::{tempdir, tempdir_in};

fn context() -> DevContext {
    DevContext {
        repository_root: PathBuf::from("/work/hephaestus"),
        local_root: PathBuf::from("/work/hephaestus/.local"),
        runtime_root: PathBuf::from("/tmp/hephaestus-runtime-test"),
        secret_runtime_root: PathBuf::from("/dev/shm/hephaestus-runtime-test"),
        namespace: String::from("hephaestus-local"),
        postgres_port: 55432,
        zot_port: 55000,
    }
}

#[test]
fn rendered_configuration_has_no_template_values() {
    let rendered = render_template(
            r#"{"storage":"{{ zot.storage_root }}","port":{{ zot.private_port }},"realm":"{{ hephaestus.registry_token_realm }}","service":"{{ hephaestus.registry_service }}","address":"{{ zot.private_address }}"}"#,
            &context(),
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        )
        .expect("render configuration");
    assert!(rendered.contains("/var/lib/registry"));
    assert!(rendered.contains("55000"));
    assert!(!rendered.contains("{{"));
}

#[test]
fn readiness_requires_the_expected_bearer_challenge() {
    let context = context();
    assert!(challenge_matches(
        &context,
        "HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Bearer realm=\"http://127.0.0.1:8080/v1/registry/token\",service=\"localhost:55000\"\r\n"
    ));
    assert!(!challenge_matches(
        &context,
        "HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Basic realm=\"local\"\r\n"
    ));
}

#[test]
#[ignore = "requires Podman, the pinned Zot image, and a loopback port"]
fn pinned_zot_starts_with_an_authenticated_challenge() {
    let local = tempdir().expect("local state");
    let secret = tempdir_in("/dev/shm").expect("secret runtime");
    let crate_directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repository_root = crate_directory
        .parent()
        .and_then(std::path::Path::parent)
        .expect("workspace root")
        .canonicalize()
        .expect("canonical workspace root");
    let context = DevContext {
        repository_root,
        local_root: local.path().to_owned(),
        runtime_root: PathBuf::from("/tmp/hephaestus-runtime-test"),
        secret_runtime_root: secret.path().to_owned(),
        namespace: format!("hephaestus-zot-test-{}", std::process::id()),
        postgres_port: 55432,
        zot_port: 55001,
    };
    start(&context).expect("start pinned Zot");
    stop(&context).expect("stop pinned Zot");
}
