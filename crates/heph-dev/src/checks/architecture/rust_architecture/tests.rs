//! Focused semantic architecture rule tests.

use super::{
    RULES,
    rules::{is_configuration_path, is_storage_path},
    source_scan::{scan_sentinel_source, should_skip_sentinel_directory, validate_source},
};
use std::path::Path;

fn all_rules() -> Vec<&'static str> {
    RULES.to_vec()
}

#[test]
fn valid_configuration_and_adapter_fixture_is_accepted() {
    let active = all_rules();
    let mut diagnostics = Vec::new();
    validate_source(
        Path::new("crates/example/src/config/settings.rs"),
        include_str!("../../../../tests/fixtures/rust-architecture/valid/src/config/settings.rs"),
        &active,
        &mut diagnostics,
    );
    validate_source(
        Path::new("crates/example/src/http/client.rs"),
        include_str!("../../../../tests/fixtures/rust-architecture/valid/src/http/client.rs"),
        &active,
        &mut diagnostics,
    );
    assert!(
        diagnostics.is_empty(),
        "unexpected diagnostics: {diagnostics:?}"
    );
}

#[test]
fn sensitive_flow_fixture_allows_opaque_conversions() {
    let active = all_rules();
    let mut diagnostics = Vec::new();
    validate_source(
        Path::new("crates/example/src/rpc/sensitive.rs"),
        include_str!("../../../../tests/fixtures/rust-architecture/valid/src/sensitive_flow.rs"),
        &active,
        &mut diagnostics,
    );
    assert!(
        diagnostics.is_empty(),
        "unexpected diagnostics: {diagnostics:?}"
    );
}

#[test]
fn nested_sensitive_flow_fixture_allows_opaque_conversions_and_safe_fields() {
    let active = all_rules();
    let mut diagnostics = Vec::new();
    validate_source(
        Path::new("crates/example/src/rpc/nested_sensitive.rs"),
        include_str!(
            "../../../../tests/fixtures/rust-architecture/valid/src/nested_sensitive_flow.rs"
        ),
        &active,
        &mut diagnostics,
    );
    assert!(
        diagnostics.is_empty(),
        "unexpected nested-flow diagnostics: {diagnostics:?}"
    );
}

#[test]
fn nested_sensitive_flow_fixture_reports_all_plaintext_sinks() {
    let active = all_rules();
    let mut diagnostics = Vec::new();
    validate_source(
        Path::new("crates/example/src/rpc/nested_sensitive.rs"),
        include_str!(
            "../../../../tests/fixtures/rust-architecture/invalid/src/nested_sensitive_flow.rs"
        ),
        &active,
        &mut diagnostics,
    );
    let flow_diagnostics = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.message.contains("sensitive request field"))
        .collect::<Vec<_>>();
    for sink in ["warn", "json", "append_application_event", "body"] {
        assert!(
            flow_diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains(sink)),
            "missing {sink} diagnostic: {diagnostics:?}"
        );
    }
    assert!(flow_diagnostics.iter().all(|diagnostic| {
        diagnostic.message.contains("source at line ")
            && diagnostic.message.contains("sink")
            && diagnostic.message.contains("line ")
    }));
    assert!(diagnostics.iter().all(|diagnostic| {
        !diagnostic.message.contains("safe nested field")
            && !diagnostic.message.contains("non-sensitive")
    }));
}

#[test]
fn sensitive_flow_fixture_reports_source_and_sink_without_plaintext() {
    let active = all_rules();
    let mut diagnostics = Vec::new();
    validate_source(
        Path::new("crates/example/src/rpc/sensitive.rs"),
        include_str!("../../../../tests/fixtures/rust-architecture/invalid/src/sensitive_flow.rs"),
        &active,
        &mut diagnostics,
    );
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.rule_id == "SEC-NO-SENSITIVE-LOG-ARGUMENTS")
    );
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.rule_id == "SEC-SENSITIVE-NO-UNRESTRICTED-FORMAT")
    );
    let flow_diagnostics = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.message.contains("sensitive request field"))
        .collect::<Vec<_>>();
    assert!(flow_diagnostics.len() >= 7, "diagnostics: {diagnostics:?}");
    assert!(
        flow_diagnostics
            .iter()
            .all(|diagnostic| diagnostic.message.contains("sink"))
    );
    assert!(flow_diagnostics.iter().all(|diagnostic| {
        diagnostic.message.contains("source at line ")
            && diagnostic.message.contains("in rejected")
            && diagnostic.message.contains("line ")
    }));
    assert!(flow_diagnostics.iter().any(|diagnostic| {
        diagnostic.message.contains("info sink")
            && diagnostic.message.contains("source at line 3")
            && diagnostic.message.contains("line 4")
    }));
    assert!(flow_diagnostics.iter().any(|diagnostic| {
        diagnostic.message.contains("format sink")
            && diagnostic.message.contains("source at line 3")
            && diagnostic.message.contains("line 5")
    }));
    for sink in [
        "info",
        "format",
        "anyhow",
        "json",
        "insert",
        "label",
        "publish",
        "body",
        "to_string",
    ] {
        assert!(
            flow_diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains(sink)),
            "missing {sink} diagnostic: {diagnostics:?}"
        );
    }
    assert!(diagnostics.iter().all(|diagnostic| {
        !diagnostic.message.contains("received handoff")
            && !diagnostic.message.contains("invalid handoff")
    }));
}

#[test]
fn migration_fingerprint_exception_is_exactly_scoped_to_its_build_script() {
    let active = all_rules();
    let source = "use std::fs; fn main() { let _ = fs::read_dir(\"migrations\"); let _ = fs::read(\"migration.sql\"); }";

    let mut build_script_diagnostics = Vec::new();
    validate_source(
        Path::new("crates/heph-core/authorization/runtime-authority-postgres/build.rs"),
        source,
        &active,
        &mut build_script_diagnostics,
    );
    assert!(build_script_diagnostics.is_empty());

    let mut neighboring_diagnostics = Vec::new();
    validate_source(
        Path::new("crates/other-adapter/build.rs"),
        source,
        &active,
        &mut neighboring_diagnostics,
    );
    assert!(
        neighboring_diagnostics
            .iter()
            .any(|diagnostic| diagnostic.rule_id == "ARCH-FILESYSTEM-ONLY-IN-ADAPTERS")
    );
}

#[test]
fn moved_topology_paths_keep_exact_boundary_classification() {
    assert!(is_configuration_path(Path::new(
        "crates/heph-std/runtime/vm/libkrun/src/worker.rs"
    )));
    for path in [
        "crates/heph-core/forge/build/orchestrator/src/lib.rs",
        "crates/heph-core/forge/release/artifact-store/src/lib.rs",
        "crates/heph-core/forge/service/src/storage.rs",
        "crates/heph-core/identity/git-credential/src/main.rs",
        "crates/heph-std/forge/registry/publisher/src/lib.rs",
        "crates/heph-std/run/runtime-local/src/lib.rs",
    ] {
        assert!(is_storage_path(Path::new(path)), "{path}");
    }
    assert!(!is_storage_path(Path::new(
        "crates/heph-core/forge/build/unrelated/src/lib.rs"
    )));
}

#[test]
fn invalid_fixture_reports_actionable_boundary_diagnostics() {
    let active = all_rules();
    let mut diagnostics = Vec::new();
    validate_source(
        Path::new("crates/example/src/application.rs"),
        include_str!("../../../../tests/fixtures/rust-architecture/invalid/src/application.rs"),
        &active,
        &mut diagnostics,
    );
    for rule in RULES
        .iter()
        .copied()
        .filter(|rule| *rule != "SEC-SENTINEL-NO-PLAINTEXT")
    {
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.rule_id == rule),
            "missing diagnostic for {rule}: {diagnostics:?}"
        );
    }
}

#[test]
fn sentinel_scan_allows_test_only_values_and_rejects_production_values() {
    let mut valid = Vec::new();
    scan_sentinel_source(
        Path::new("crates/example/src/lib.rs"),
        include_str!("../../../../tests/fixtures/secret-safety/valid/src/lib.rs"),
        &mut valid,
    );
    assert!(valid.is_empty(), "unexpected diagnostics: {valid:?}");

    let mut invalid = Vec::new();
    scan_sentinel_source(
        Path::new("crates/example/src/lib.rs"),
        include_str!("../../../../tests/fixtures/secret-safety/invalid/src/lib.rs"),
        &mut invalid,
    );
    assert_eq!(invalid.len(), 1);
    assert_eq!(invalid[0].rule_id, "SEC-SENTINEL-NO-PLAINTEXT");
}

#[test]
fn sentinel_scan_skips_private_local_state() {
    assert!(should_skip_sentinel_directory(Path::new(".local")));
    assert!(!should_skip_sentinel_directory(Path::new("crates/example")));
}

#[test]
fn example_vendor_exclusion_keeps_application_sources_scanned() {
    for path in [
        "examples/cooking/cooking-gateway/vendor/memchr/src",
        "examples/cooking/cooking-gateway/target/debug",
    ] {
        assert!(should_skip_sentinel_directory(Path::new(path)));
    }
    let application = Path::new("examples/cooking/cooking-gateway/src/main.rs");
    assert!(!should_skip_sentinel_directory(application));
    assert!(!should_skip_sentinel_directory(Path::new(
        "examples/cooking/cooking-gateway/vendor-like"
    )));
    let mut diagnostics = Vec::new();
    scan_sentinel_source(
        application,
        "const LEAK: &str = \"secret-sentinel\";",
        &mut diagnostics,
    );
    assert_eq!(diagnostics.len(), 1);
}
