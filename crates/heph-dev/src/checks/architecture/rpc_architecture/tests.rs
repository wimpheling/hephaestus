use super::{RULES, validate_source, validate_source_with_context};
use crate::checks::architecture::Diagnostic;
use std::{fs, path::Path};
use tempfile::tempdir;

const INVALID: &str =
    include_str!("../../../../tests/fixtures/rpc-architecture/invalid/src/rpc/service.rs");
const INWARD: &str =
    include_str!("../../../../tests/fixtures/rpc-architecture/invalid/src/domain/service.rs");
const VALID: &str =
    include_str!("../../../../tests/fixtures/rpc-architecture/valid/src/rpc/error.rs");

fn active() -> Vec<&'static str> {
    RULES
        .into_iter()
        .chain(["RPC-GENERATED-TYPES-DO-NOT-LEAK-INWARD"])
        .collect()
}

#[test]
fn invalid_rpc_fixture_covers_layout_routes_errors_and_handler_io() {
    let mut diagnostics = Vec::new();
    validate_source(
        Path::new("crates/example/src/rpc/service.rs"),
        INVALID,
        &active(),
        &mut diagnostics,
    );
    for rule in [
        "RPC-METHOD-IN-SEPARATE-FILE",
        "RPC-NON_RPC-HTTP-ALLOWLIST",
        "RPC-NO-DIRECT-CONNECT-ERROR",
        "RPC-HANDLER-IS-THIN",
    ] {
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.rule_id == rule),
            "fixture did not trigger {rule}"
        );
    }
}

#[test]
fn nested_service_mod_is_checked_as_a_handler() {
    let mut diagnostics = Vec::new();
    validate_source(
        Path::new("crates/example/src/rpc/identity/mod.rs"),
        "async fn resolve() { let _ = sqlx::query(\"SELECT 1\"); }",
        &active(),
        &mut diagnostics,
    );
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.rule_id == "RPC-HANDLER-IS-THIN")
    );
}

#[test]
fn inward_fixture_rejects_connect_generated_types_and_transport_errors() {
    let mut diagnostics = Vec::new();
    validate_source(
        Path::new("crates/example/src/domain/service.rs"),
        INWARD,
        &active(),
        &mut diagnostics,
    );
    for rule in [
        "RPC-CONNECT-ONLY-IN-TRANSPORT",
        "RPC-ERRORS-MAPPED-AT-BOUNDARY",
        "RPC-GENERATED-TYPES-DO-NOT-LEAK-INWARD",
    ] {
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.rule_id == rule),
            "fixture did not trigger {rule}"
        );
    }
}

#[test]
fn central_error_adapter_and_health_route_are_allowed() {
    let mut diagnostics = Vec::<Diagnostic>::new();
    validate_source(
        Path::new("crates/example/src/rpc/error.rs"),
        VALID,
        &active(),
        &mut diagnostics,
    );
    assert!(diagnostics.is_empty());
}

#[test]
fn runtime_and_ui_git_transport_routes_are_allowlisted_exactly() {
    let mut diagnostics = Vec::new();
    validate_source(
        Path::new("crates/heph-app/src/runtime_git_listener.rs"),
        r#"
                let _router = axum::Router::new()
                    .route("/", axum::routing::get(handler))
                    .route("/_heph/git/{repository}/info/refs", axum::routing::get(handler))
                    .route("/_heph/git/{repository}/git-upload-pack", axum::routing::post(handler))
                    .route("/_heph/git/{repository}/git-receive-pack", axum::routing::post(handler));
            "#,
        &active(),
        &mut diagnostics,
    );
    assert!(diagnostics.is_empty());

    let mut diagnostics = Vec::new();
    validate_source(
        Path::new("crates/heph-app/src/git_transport.rs"),
        r#"let _router = axum::Router::new().route("/", axum::routing::get(handler));"#,
        &active(),
        &mut diagnostics,
    );
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.rule_id == "RPC-NON_RPC-HTTP-ALLOWLIST")
    );

    let mut diagnostics = Vec::new();
    validate_source(
        Path::new("crates/heph-app/src/git_transport.rs"),
        r#"let _router = axum::Router::new().route("/_heph/git/{repository}/other", axum::routing::get(handler));"#,
        &active(),
        &mut diagnostics,
    );
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.rule_id == "RPC-NON_RPC-HTTP-ALLOWLIST")
    );
}

#[test]
fn rpc_module_helpers_cannot_hide_database_io() {
    let mut diagnostics = Vec::new();
    validate_source(
        Path::new("crates/example/src/rpc/mod.rs"),
        "fn helper() { let _query = sqlx::query(\"SELECT 1\"); }",
        &active(),
        &mut diagnostics,
    );
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.rule_id == "RPC-HANDLER-IS-THIN")
    );
}

#[test]
fn nested_rpc_services_cannot_hide_database_io() {
    let mut diagnostics = Vec::new();
    validate_source(
        Path::new("crates/example/src/rpc/mod.rs"),
        "mod nested { fn load() { let _query = sqlx::query_as::<_, (i64,)>(\"SELECT 1\"); } }",
        &active(),
        &mut diagnostics,
    );
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.rule_id == "RPC-HANDLER-IS-THIN")
    );
}

#[test]
fn rpc_constructor_type_plumbing_may_carry_a_pool() {
    let mut diagnostics = Vec::new();
    validate_source(
        Path::new("crates/example/src/rpc/service.rs"),
        "use sqlx::PgPool; struct Service(PgPool); impl Service { fn new(pool: PgPool) -> Self { Self(pool) } }",
        &active(),
        &mut diagnostics,
    );
    assert!(diagnostics.is_empty());
}

#[test]
fn event_transport_may_encode_the_generated_product_envelope() {
    let mut diagnostics = Vec::<Diagnostic>::new();
    validate_source(
        Path::new("crates/example/src/event_adapter.rs"),
        "use rpc_proto::messages::hephaestus::event::v1::ProductEvent;",
        &active(),
        &mut diagnostics,
    );
    assert!(diagnostics.is_empty());
}

#[test]
fn cfg_test_parent_marks_path_attribute_descendants_as_test_only() {
    let directory = tempdir().expect("fixture root");
    let source_root = directory.path().join("crates/example/src/rpc/event");
    fs::create_dir_all(source_root.join("watch/tests")).expect("fixture directories");
    fs::write(
        source_root.join("watch.rs"),
        "#[cfg(test)]\n#[path = \"watch/tests.rs\"]\nmod tests;\n",
    )
    .expect("watch module");
    fs::write(
        source_root.join("watch/tests.rs"),
        "#[path = \"tests/connect.rs\"]\nmod connect;\n",
    )
    .expect("test module");
    let connect = source_root.join("watch/tests/connect.rs");
    fs::write(
        &connect,
        "fn fixture() { let _ = sqlx::query(\"SELECT 1\"); }",
    )
    .expect("fixture source");

    assert!(super::module_graph::is_test_only_source(
        directory.path(),
        &connect
    ));
    let mut diagnostics = Vec::new();
    validate_source_with_context(
        Path::new("crates/example/src/rpc/event/tests/connect.rs"),
        "fn fixture() { let _ = sqlx::query(\"SELECT 1\"); }",
        &active(),
        &mut diagnostics,
        true,
    );
    assert!(
        !diagnostics
            .iter()
            .any(|diagnostic| diagnostic.rule_id == "RPC-HANDLER-IS-THIN")
    );

    fs::write(
        source_root.join("watch.rs"),
        "#[path = \"watch/tests.rs\"]\nmod tests;\n",
    )
    .expect("non-test watch module");
    assert!(!super::module_graph::is_test_only_source(
        directory.path(),
        &connect
    ));
    let mut diagnostics = Vec::new();
    validate_source_with_context(
        Path::new("crates/example/src/rpc/event/tests/connect.rs"),
        "fn fixture() { let _ = sqlx::query(\"SELECT 1\"); }",
        &active(),
        &mut diagnostics,
        super::module_graph::is_test_only_source(directory.path(), &connect),
    );
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.rule_id == "RPC-HANDLER-IS-THIN")
    );
}

#[test]
fn package_integration_helpers_are_test_only_but_src_tests_are_production() {
    let directory = tempdir().expect("fixture root");
    let package = directory.path().join("crates/example");
    fs::create_dir_all(package.join("tests/composition/ui_installation_rpc"))
        .expect("integration fixture");
    fs::create_dir_all(package.join("src/rpc/tests")).expect("production fixture");
    fs::write(
        package.join("Cargo.toml"),
        "[package]\nname = \"example\"\nversion = \"0.1.0\"\n",
    )
    .expect("package manifest");
    fs::write(
        package.join("tests/composition/ui_installation_rpc.rs"),
        "#[path = \"ui_installation_rpc/seed_base.rs\"]\nmod seed_base;\n",
    )
    .expect("integration wrapper");
    let integration_helper = package.join("tests/composition/ui_installation_rpc/seed_base.rs");
    let production_helper = package.join("src/rpc/tests/seed.rs");
    fs::write(
        &integration_helper,
        "fn seed() { let _ = sqlx::query(\"SELECT 1\"); }",
    )
    .expect("integration source");
    fs::write(
        &production_helper,
        "fn seed() { let _ = sqlx::query(\"SELECT 1\"); }",
    )
    .expect("production source");

    let integration_test_only =
        super::module_graph::is_test_only_source(directory.path(), &integration_helper);
    assert!(integration_test_only);
    let production_test_only =
        super::module_graph::is_test_only_source(directory.path(), &production_helper);
    assert!(!production_test_only);

    let mut diagnostics = Vec::new();
    validate_source_with_context(
        Path::new("crates/example/tests/composition/ui_installation_rpc/seed_base.rs"),
        "fn seed() { let _ = sqlx::query(\"SELECT 1\"); }",
        &active(),
        &mut diagnostics,
        integration_test_only,
    );
    assert!(
        !diagnostics
            .iter()
            .any(|diagnostic| diagnostic.rule_id == "RPC-HANDLER-IS-THIN")
    );

    let mut diagnostics = Vec::new();
    validate_source_with_context(
        Path::new("crates/example/src/rpc/tests/seed.rs"),
        "fn seed() { let _ = sqlx::query(\"SELECT 1\"); }",
        &active(),
        &mut diagnostics,
        production_test_only,
    );
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.rule_id == "RPC-HANDLER-IS-THIN")
    );
}
