use super::{scan_repository_sentinels, scan_sentinel_source};
use std::{fs, path::Path};
use tempfile::tempdir;

fn assert_inline(source: &str, expected_line: Option<usize>) {
    let mut diagnostics = Vec::new();
    scan_sentinel_source(
        Path::new("crates/example/src/lib.rs"),
        source,
        &mut diagnostics,
    );
    assert_eq!(diagnostics.len(), usize::from(expected_line.is_some()));
    if let Some(line) = expected_line {
        assert_eq!(diagnostics[0].rule_id, "SEC-SENTINEL-NO-PLAINTEXT");
        assert!(diagnostics[0].message.contains(&format!("lib.rs:{line} ")));
    }
}

fn assert_child(parent: &str, intermediate: &str, child: &str, denied: bool) {
    let directory = tempdir().expect("fixture root");
    let source = directory.path().join("crates/example/src");
    fs::create_dir_all(source.join("owner/cases")).expect("fixture directories");
    fs::write(
        source.parent().expect("package").join("Cargo.toml"),
        "[package]\nname = \"example\"\n",
    )
    .expect("package manifest");
    fs::write(source.join("owner.rs"), parent).expect("module registration");
    fs::write(source.join("owner/cases.rs"), intermediate).expect("intermediate module");
    fs::write(source.join("owner/cases/child.rs"), child).expect("child module");
    let mut diagnostics = Vec::new();
    scan_repository_sentinels(directory.path(), &mut diagnostics);
    assert_eq!(diagnostics.len(), usize::from(denied));
    if denied {
        assert_eq!(diagnostics[0].rule_id, "SEC-SENTINEL-NO-PLAINTEXT");
        assert!(diagnostics[0].message.contains("owner/cases/child.rs:1 "));
    }
}

const CHILD: &str = "const MARKER: &str = \"secret-sentinel\";";
const INTERMEDIATE: &str = "#[path = \"cases/child.rs\"]\nmod child;\n";

#[test]
fn inherited_explicit_test_path_children_are_allowed() {
    assert_child(
        "#[cfg(test)]\n#[path = \"owner/cases.rs\"]\nmod cases;\n",
        INTERMEDIATE,
        CHILD,
        false,
    );
}

#[test]
fn split_test_and_feature_attributes_preserve_inherited_test_context() {
    assert_child(
        "#[cfg(test)]\n#[cfg(feature = \"test-fixtures\")]\n#[path = \"owner/cases.rs\"]\nmod cases;\n",
        INTERMEDIATE,
        CHILD,
        false,
    );
}

#[test]
fn unqualified_external_child_is_production() {
    assert_child(
        "#[path = \"owner/cases.rs\"]\nmod cases;\n",
        INTERMEDIATE,
        CHILD,
        true,
    );
}

#[test]
fn any_test_or_feature_parent_is_not_test_only() {
    assert_child(
        "#[cfg(any(test, feature = \"fixture\"))]\n#[path = \"owner/cases.rs\"]\nmod cases;\n",
        INTERMEDIATE,
        CHILD,
        true,
    );
}

#[test]
fn not_test_parent_is_not_test_only() {
    assert_child(
        "#[cfg(not(test))]\n#[path = \"owner/cases.rs\"]\nmod cases;\n",
        INTERMEDIATE,
        CHILD,
        true,
    );
}

#[test]
fn malformed_parent_cannot_prove_test_context() {
    assert_child(
        "#[cfg(test)]\n#[path = \"owner/cases.rs\"]\nmod cases;\nfn broken(\n",
        INTERMEDIATE,
        CHILD,
        true,
    );
}

#[test]
fn raw_string_module_registration_cannot_prove_test_context() {
    assert_child(
        "const TEXT: &str = r#\"\n#[cfg(test)]\n#[path = \"owner/cases.rs\"]\nmod cases;\n\"#;\n",
        INTERMEDIATE,
        CHILD,
        true,
    );
}

#[test]
fn invalid_child_remains_scanned_despite_test_only_parent() {
    assert_child(
        "#[cfg(test)]\n#[path = \"owner/cases.rs\"]\nmod cases;\n",
        INTERMEDIATE,
        "const MARKER: &str = \"secret-sentinel\"; fn broken(",
        true,
    );
}

#[test]
fn production_file_named_tests_is_not_exempt() {
    let directory = tempdir().expect("fixture root");
    let source = directory.path().join("crates/example/src");
    fs::create_dir_all(&source).expect("source directory");
    fs::write(source.join("tests.rs"), CHILD).expect("production source");
    let mut diagnostics = Vec::new();
    scan_repository_sentinels(directory.path(), &mut diagnostics);
    assert_eq!(diagnostics.len(), 1);
    assert!(diagnostics[0].message.contains("src/tests.rs:1 "));
}

#[test]
fn exact_inline_test_item_scope_is_allowed() {
    assert_inline(
        "#[cfg(test)]\nmod fixture { const MARKER: &str = \"secret-sentinel\"; }\n",
        None,
    );
}

#[test]
fn production_after_inline_test_module_is_denied() {
    assert_inline(
        "#[cfg(test)]\nmod fixture { const MARKER: &str = \"test-sentinel\"; }\nconst VALUE: &str = \"production-sentinel\";\n",
        Some(3),
    );
}

#[test]
fn production_after_same_line_test_item_is_denied() {
    assert_inline(
        "#[cfg(test)] mod fixture { const MARKER: &str = \"test-sentinel\"; } const VALUE: &str = \"production-sentinel\";",
        Some(1),
    );
}

#[test]
fn any_test_or_feature_inline_item_is_denied() {
    assert_inline(
        "#[cfg(any(test, feature = \"fixture\"))]\nconst MARKER: &str = \"secret-sentinel\";",
        Some(2),
    );
}

#[test]
fn comment_attribute_cannot_hide_production_marker() {
    assert_inline(
        "// #[cfg(test)]\nconst MARKER: &str = \"secret-sentinel\";",
        Some(2),
    );
}

#[test]
fn invalid_inline_source_has_no_test_exemption() {
    assert_inline(
        "#[cfg(test)] mod fixture { const MARKER: &str = \"secret-sentinel\";",
        Some(1),
    );
}

#[test]
fn exact_inline_test_impl_method_scope_is_allowed() {
    assert_inline(
        "struct Fixture; impl Fixture { #[cfg(test)] fn inspect() { let _ = \"test-sentinel\"; } }",
        None,
    );
}

#[test]
fn variant_exact_test_scope_is_allowed() {
    assert_inline(
        "enum Fixture { #[cfg(test)] TestOwner(OwnerSentinel), Production }",
        None,
    );
}

#[test]
fn variant_production_neighbor_is_denied() {
    assert_inline(
        "enum Fixture {\n#[cfg(test)]\nTestOwner(OwnerSentinel),\nProductionSentinel,\n}",
        Some(4),
    );
}

#[test]
fn variant_any_test_or_feature_is_denied() {
    assert_inline(
        "enum Fixture {\n#[cfg(any(test, feature = \"fixture\"))]\nTestOwner(OwnerSentinel),\n}",
        Some(3),
    );
}
