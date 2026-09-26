use super::{
    RULE,
    inventory::{PoolBinding, PoolField, validate_pool_inventory},
    source_scan::validate_file,
};
use std::{fs, path::Path};

fn scan(name: &str, binding: PoolBinding) -> Vec<super::super::Diagnostic> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/db-rls");
    let relative = Path::new(name).join("src/lib.rs");
    let path = root.join(&relative);
    let source = fs::read_to_string(&path).expect("fixture source");
    let file = syn::parse_file(&source).expect("fixture Rust");
    let mut diagnostics = Vec::new();
    validate_file(&file, &relative, &path, &[binding], &root, &mut diagnostics);
    diagnostics
}

fn binding() -> PoolBinding {
    PoolBinding {
        source: "fixture",
        owner: "AppStore",
        field: "app_pool",
    }
}

#[test]
fn an_unlisted_pool_field_is_rejected_by_the_structural_inventory() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/db-rls");
    let relative = Path::new("invalid/new_pool_field/src/lib.rs");
    let path = root.join(relative);
    let source = fs::read_to_string(&path).expect("fixture source");
    let file = syn::parse_file(&source).expect("fixture Rust");
    let mut diagnostics = Vec::new();
    let inventory = [PoolField {
        source: "invalid/new_pool_field/src/lib.rs",
        owner: "AppStore",
        field: "app_pool",
        application_role: true,
    }];
    let bindings = [PoolBinding {
        source: "invalid/new_pool_field/src/lib.rs",
        owner: "AppStore",
        field: "app_pool",
    }];
    validate_pool_inventory(&file, relative, &inventory, &bindings, &mut diagnostics);
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.rule_id == RULE)
    );
}

#[test]
fn valid_context_fixtures_are_clean() {
    for fixture in [
        "valid/canonical",
        "valid/repeatable",
        "valid/bootstrap",
        "valid/worker",
    ] {
        assert!(
            scan(fixture, binding())
                .iter()
                .all(|diagnostic| diagnostic.rule_id != RULE),
            "{fixture}"
        );
    }
}

#[test]
fn invalid_context_fixtures_report_the_rule() {
    for fixture in [
        "invalid/direct",
        "invalid/unscoped",
        "invalid/misleading",
        "invalid/pre_context",
        "invalid/incomplete_helper",
        "invalid/comment_bypass",
        "invalid/unbacked_resolver",
    ] {
        assert!(
            scan(fixture, binding())
                .iter()
                .any(|diagnostic| diagnostic.rule_id == RULE),
            "{fixture}"
        );
    }
}
