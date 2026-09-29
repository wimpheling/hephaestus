use super::super::STATIC_RULE;
use super::support::{scan_dynamic_queries, scope_result, static_exception};

#[test]
fn item_exception_is_exact_and_does_not_cover_neighbor_or_other_rule() {
    let source = r"
use sqlx::query;
fn allowed(value: String) { query(&value); }
fn neighbor(value: String) { query(&value); }
";
    let diagnostics = scan_dynamic_queries(
        source,
        &[
            static_exception("src/lib.rs#allowed", STATIC_RULE),
            static_exception("src/lib.rs#neighbor", "DB-SQLX-ONLY-IN-POSTGRES-ADAPTERS"),
            static_exception("other.rs#allowed", STATIC_RULE),
        ],
    );
    assert_eq!(
        diagnostics.len(),
        1,
        "neighboring dynamic query must remain reported"
    );
    assert_eq!(diagnostics[0].rule_id, STATIC_RULE);
}

#[test]
fn line_exception_matches_only_the_exact_query_line() {
    let source = "use sqlx::query;\nfn first(value: String) { query(&value); }\nfn second(value: String) { query(&value); }\n";
    let diagnostics =
        scan_dynamic_queries(source, &[static_exception("src/lib.rs:2", STATIC_RULE)]);
    assert_eq!(
        diagnostics.len(),
        1,
        "the query on the other line remains reported"
    );
}

#[test]
fn impl_method_exception_uses_the_qualified_item_name() {
    let source = r"
use sqlx::query;
struct Database;
impl Database {
    fn create(value: String) { query(&value); }
    fn neighbor(value: String) { query(&value); }
}
";
    let diagnostics = scan_dynamic_queries(
        source,
        &[static_exception("src/lib.rs#Database::create", STATIC_RULE)],
    );
    assert_eq!(
        diagnostics.len(),
        1,
        "the neighboring method remains reported"
    );
}

#[test]
fn same_named_functions_require_module_qualified_selectors() {
    let source = r"
use sqlx::query;
mod one { pub fn allowed(value: String) { query(&value); } }
mod two { pub fn allowed(value: String) { query(&value); } }
";
    assert!(scope_result(source, "src/lib.rs#allowed").is_err());
    let diagnostics = scan_dynamic_queries(
        source,
        &[static_exception("src/lib.rs#allowed", STATIC_RULE)],
    );
    assert_eq!(diagnostics.len(), 2);
    let diagnostics = scan_dynamic_queries(
        source,
        &[static_exception("src/lib.rs#one::allowed", STATIC_RULE)],
    );
    assert_eq!(
        diagnostics.len(),
        1,
        "module-qualified selector suppresses one item"
    );
}

#[test]
fn same_named_methods_require_module_and_type_qualified_selectors() {
    let source = r"
use sqlx::query;
mod one { struct Thing; impl Thing { fn run(value: String) { query(&value); } } }
mod two { struct Thing; impl Thing { fn run(value: String) { query(&value); } } }
";
    assert!(scope_result(source, "src/lib.rs#Thing::run").is_err());
    let diagnostics = scan_dynamic_queries(
        source,
        &[static_exception("src/lib.rs#Thing::run", STATIC_RULE)],
    );
    assert_eq!(diagnostics.len(), 2);
    let diagnostics = scan_dynamic_queries(
        source,
        &[static_exception("src/lib.rs#one::Thing::run", STATIC_RULE)],
    );
    assert_eq!(
        diagnostics.len(),
        1,
        "fully qualified method suppresses one item"
    );
}

#[test]
fn outer_function_exception_does_not_cover_nested_function() {
    let source = r"
use sqlx::query;
fn outer(value: String) { query(&value); fn nested(value: String) { query(&value); } }
";
    let diagnostics =
        scan_dynamic_queries(source, &[static_exception("src/lib.rs#outer", STATIC_RULE)]);
    assert_eq!(diagnostics.len(), 1);
    assert!(scope_result(source, "src/lib.rs#outer::nested").is_ok());
}

#[test]
fn nonexistent_qualified_method_is_rejected_without_cross_type_matching() {
    let source = r"
struct A;
impl A { fn run(value: String) { let _ = value; } }
struct B;
impl B { fn other(value: String) { let _ = value; } }
";
    assert!(scope_result(source, "src/lib.rs#A::other").is_err());
}
