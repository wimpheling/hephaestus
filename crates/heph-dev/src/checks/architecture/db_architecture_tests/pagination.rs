use super::super::PAGINATION_RULE;
use super::support::{
    scan_pagination_fixture, scan_pagination_source, scan_pagination_source_with_migration,
};

#[test]
fn valid_pagination_contract_matches_order_cursor_and_tie_breaker() {
    assert!(scan_pagination_fixture("valid/adapter-postgres").is_empty());
}

#[test]
fn pagination_contract_reports_missing_order() {
    let diagnostics = scan_pagination_fixture("invalid/pagination-missing-order");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.rule_id == PAGINATION_RULE
            && diagnostic.message.contains("no ORDER BY clause")
            && diagnostic.message.contains("list")
    }));
}

#[test]
fn pagination_contract_reports_mismatched_cursor_keys() {
    let diagnostics = scan_pagination_fixture("invalid/pagination-mismatched-cursor");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.rule_id == PAGINATION_RULE
            && diagnostic.message.contains("cursor keys")
            && diagnostic.message.contains("list")
    }));
}

#[test]
fn pagination_contract_reports_missing_unique_tie_breaker() {
    let diagnostics = scan_pagination_fixture("invalid/pagination-missing-tie-breaker");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.rule_id == PAGINATION_RULE
            && diagnostic.message.contains("unique_tie_breaker")
            && diagnostic.message.contains("list")
    }));
}

#[test]
fn undeclared_canonical_page_query_is_rejected() {
    let diagnostics = scan_pagination_source(
        "fn list(page: i64) { let _ = sqlx::query(\"SELECT id FROM items WHERE id > $1 LIMIT $2\"); let _ = page; }",
        None,
    );
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.rule_id == PAGINATION_RULE
            && diagnostic
                .message
                .contains("requires a pagination.toml declaration")
    }));
}

#[test]
fn undeclared_descending_scalar_page_query_is_rejected() {
    let diagnostics = scan_pagination_fixture("invalid/pagination-undeclared-desc-scalar");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.rule_id == PAGINATION_RULE
            && diagnostic
                .message
                .contains("requires a pagination.toml declaration")
    }));
}

#[test]
fn undeclared_descending_tuple_page_query_is_rejected() {
    let diagnostics = scan_pagination_fixture("invalid/pagination-undeclared-desc-tuple");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.rule_id == PAGINATION_RULE
            && diagnostic
                .message
                .contains("requires a pagination.toml declaration")
    }));
}

#[test]
fn stale_pagination_contract_is_rejected() {
    let diagnostics = scan_pagination_source(
        "fn list(page: i64) { let _ = page; }",
        Some(
            "[[queries]]\nitem = \"missing\"\nquery_index = 1\norder = [\"id ASC\"]\ncursor_keys = [\"id\"]\ncursor_operator = \">\"\nunique_tie_breaker = \"id\"\nunique_keys = [\"id\"]\n",
        ),
    );
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.rule_id == PAGINATION_RULE
            && diagnostic.message.contains("targets no SQLx query")
    }));
}

#[test]
fn declared_page_query_requires_a_limit() {
    let diagnostics = scan_pagination_source(
        "fn list(page: i64) { let _ = sqlx::query(\"SELECT id FROM items WHERE id > $1\"); let _ = page; }",
        Some(
            "[[queries]]\nitem = \"list\"\nquery_index = 1\norder = [\"id ASC\"]\ncursor_keys = [\"id\"]\ncursor_operator = \">\"\nunique_tie_breaker = \"id\"\nunique_keys = [\"id\"]\n",
        ),
    );
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.rule_id == PAGINATION_RULE && diagnostic.message.contains("requires LIMIT")
    }));
}

#[test]
fn uuid_row_lookup_mode_rejects_scalar_cursor_predicates() {
    let diagnostics = scan_pagination_source(
        "fn list(page: i64) { let _ = sqlx::query(\"SELECT id FROM items WHERE id > $1 LIMIT $2\"); let _ = page; }",
        Some(
            "[[queries]]\nitem = \"list\"\nquery_index = 1\norder = [\"id ASC\"]\ncursor_keys = [\"id\"]\ncursor_operator = \">\"\nunique_tie_breaker = \"id\"\nunique_keys = [\"id\"]\ncursor_mode = \"uuid_row_lookup\"\n",
        ),
    );
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.rule_id == PAGINATION_RULE && diagnostic.message.contains("uuid_row_lookup")
    }));
}

#[test]
fn mixed_order_directions_are_rejected_without_a_matching_operator_contract() {
    let diagnostics = scan_pagination_source(
        "fn list(page: i64) { let _ = sqlx::query(\"SELECT id FROM items WHERE (items.name, items.id) > ($1, $2) ORDER BY items.name ASC, items.id DESC LIMIT $3\"); let _ = page; }",
        Some(
            "[[queries]]\nitem = \"list\"\nquery_index = 1\norder = [\"items.name ASC\", \"items.id DESC\"]\ncursor_keys = [\"items.name\", \"items.id\"]\ncursor_operator = \">\"\nunique_tie_breaker = \"items.id\"\nunique_keys = [\"items.id\"]\n",
        ),
    );
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.rule_id == PAGINATION_RULE
            && diagnostic.message.contains("uniform ORDER BY direction")
    }));
}

#[test]
fn stored_function_contract_is_pinned_to_migration_cursor_and_limit() {
    let declaration = "[[queries]]\nitem = \"inspect\"\nquery_index = 1\norder = [\"audit.id ASC\"]\ncursor_keys = [\"audit.id\"]\ncursor_operator = \">\"\nunique_tie_breaker = \"audit.id\"\nunique_keys = [\"audit.id\"]\ncursor_mode = \"stored_function\"\n";
    let source = "fn inspect(page: i64) { let _ = sqlx::query(\"SELECT * FROM inspect_run_https_uses($1, $2, $3)\"); let _ = page; }";
    let migration = "CREATE FUNCTION inspect_run_https_uses(target_run uuid, after_id uuid, page_size integer) RETURNS TABLE (id uuid) AS $$ SELECT id FROM audit WHERE (after_id IS NULL OR audit.id > after_id) ORDER BY audit.id LIMIT greatest(1, least(page_size, 201)) $$;";
    assert!(
        scan_pagination_source_with_migration(source, Some(declaration), Some(migration))
            .is_empty()
    );
    let changed_migration = migration.replace("least(page_size, 201)", "page_size");
    let diagnostics =
        scan_pagination_source_with_migration(source, Some(declaration), Some(&changed_migration));
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.rule_id == PAGINATION_RULE && diagnostic.message.contains("migration 0061")
    }));
}
