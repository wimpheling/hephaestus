use super::{
    Diagnostic, PAGINATION_RULE, PaginationContract, SqlVisitor, cursor_predicate_matches,
    is_paginated_sql, normalize_sql, parse_order_by, parse_order_key, uuid_row_lookup_matches,
};
use std::{collections::BTreeSet, fs};

impl SqlVisitor<'_> {
    pub(super) fn validate_pagination(&mut self, sql: &str, query_index: usize) {
        if !self.active.contains(PAGINATION_RULE) {
            return;
        }
        let item_path = self.current_item.clone();
        let Some(contract) =
            self.pagination
                .get(item_path.as_deref(), query_index)
                .map(|contract| PaginationContract {
                    item: contract.item.clone(),
                    query_index: contract.query_index,
                    order: contract.order.clone(),
                    cursor_keys: contract.cursor_keys.clone(),
                    cursor_operator: contract.cursor_operator.clone(),
                    unique_tie_breaker: contract.unique_tie_breaker.clone(),
                    unique_keys: contract.unique_keys.clone(),
                    cursor_mode: contract.cursor_mode.clone(),
                })
        else {
            if self.has_pagination_parameter && is_paginated_sql(sql) {
                self.fail_pagination(
                    item_path.as_deref(),
                    query_index,
                    "paginated SQL requires a pagination.toml declaration",
                );
            }
            return;
        };
        self.validate_declared_pagination(sql, &contract);
    }

    pub(super) fn validate_declared_pagination(
        &mut self,
        sql: &str,
        contract: &PaginationContract,
    ) {
        let item = contract.item.clone();
        let unique_keys = &contract.unique_keys;
        let tie_breaker = &contract.unique_tie_breaker;
        let cursor_mode = &contract.cursor_mode;
        let fail = |message: String, diagnostics: &mut Vec<Diagnostic>| {
            diagnostics.push(Diagnostic::new(
                PAGINATION_RULE,
                format!(
                    "pagination contract {item} in {}: {message}",
                    self.path.display()
                ),
            ));
        };

        if cursor_mode == "stored_function" {
            if !self.inspect_run_https_uses_contract_is_intact(sql) {
                fail(
                    String::from(
                        "stored_function contract must match migration 0061 inspect_run_https_uses signature, cursor predicate, ORDER BY audit.id, and bounded LIMIT",
                    ),
                    self.diagnostics,
                );
            }
            return;
        }
        if !is_paginated_sql(sql) {
            fail(
                String::from(
                    "declared query is not canonical paginated SQL (requires LIMIT and a cursor predicate)",
                ),
                self.diagnostics,
            );
            return;
        }
        if !unique_keys.iter().any(|key| key == tie_breaker) {
            fail(
                format!("unique_tie_breaker `{tie_breaker}` has no explicit uniqueness metadata"),
                self.diagnostics,
            );
        }
        if cursor_mode == "uuid_row_lookup" && !uuid_row_lookup_matches(sql) {
            fail(
                String::from("uuid_row_lookup cursor_mode requires a cursor row lookup by UUID"),
                self.diagnostics,
            );
        }

        self.validate_order_contract(sql, contract, &fail);
    }

    pub(super) fn validate_order_contract(
        &mut self,
        sql: &str,
        contract: &PaginationContract,
        fail: &dyn Fn(String, &mut Vec<Diagnostic>),
    ) {
        let order = &contract.order;
        let cursor_keys = &contract.cursor_keys;
        let cursor_operator = &contract.cursor_operator;
        let tie_breaker = &contract.unique_tie_breaker;
        let cursor_mode = &contract.cursor_mode;
        let parsed_order = match parse_order_by(sql) {
            Ok(order) => order,
            Err(message) => {
                fail(message, self.diagnostics);
                return;
            }
        };
        let declared_order = match order
            .iter()
            .map(|value| parse_order_key(value))
            .collect::<Result<Vec<_>, _>>()
        {
            Ok(order) => order,
            Err(message) => {
                fail(message, self.diagnostics);
                return;
            }
        };
        if parsed_order != declared_order {
            fail(
                format!(
                    "declared ORDER BY {declared_order:?} does not match SQL ORDER BY {parsed_order:?}"
                ),
                self.diagnostics,
            );
        }
        let order_keys = declared_order
            .iter()
            .map(|key| key.key.clone())
            .collect::<Vec<_>>();
        if *cursor_keys != order_keys {
            fail(
                format!(
                    "cursor keys {cursor_keys:?} must exactly match ORDER BY keys {order_keys:?}"
                ),
                self.diagnostics,
            );
        }
        if order_keys.last().map(String::as_str) != Some(tie_breaker) {
            fail(
                format!("unique_tie_breaker `{tie_breaker}` must be the final ORDER BY key"),
                self.diagnostics,
            );
        }
        let directions = declared_order
            .iter()
            .map(|key| key.direction.as_str())
            .collect::<BTreeSet<_>>();
        let expected_operator = if directions.len() == 1 {
            match directions.first().copied() {
                Some("ASC") => Some(">"),
                Some("DESC") => Some("<"),
                _ => None,
            }
        } else {
            None
        };
        if expected_operator != Some(cursor_operator) {
            fail(
                format!(
                    "cursor_operator `{cursor_operator}` does not match the uniform ORDER BY direction"
                ),
                self.diagnostics,
            );
        }
        if !cursor_predicate_matches(sql, cursor_keys, cursor_operator, cursor_mode) {
            fail(
                format!(
                    "SQL cursor predicate does not compare {cursor_keys:?} with `{cursor_operator}`"
                ),
                self.diagnostics,
            );
        }
    }

    pub(super) fn inspect_run_https_uses_contract_is_intact(&self, sql: &str) -> bool {
        // Migration 0061 owns this function's SQL body. Keep the Rust escape
        // hatch pinned to its public signature and the stable cursor/limit
        // markers so a migration edit cannot silently invalidate the page.
        if normalize_sql(sql) != "select * from inspect_run_https_uses($1,$2,$3)" {
            return false;
        }
        let migration = self
            .repository_root
            .join("migrations/0061_run_provenance_inspection.sql");
        let Ok(source) = fs::read_to_string(migration) else {
            return false;
        };
        let normalized = normalize_sql(&source);
        normalized.contains(
            "create function inspect_run_https_uses(target_run uuid,after_id uuid,page_size integer)",
        ) && (normalized.contains("and (after_id is null or audit.id > after_id)")
            || normalized.contains("where (after_id is null or audit.id > after_id)"))
            && normalized.contains("order by audit.id")
            && normalized.contains("limit greatest(1,least(page_size,201))")
    }
}
