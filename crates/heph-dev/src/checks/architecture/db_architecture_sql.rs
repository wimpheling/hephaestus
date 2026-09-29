#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct OrderKey {
    pub(super) key: String,
    pub(super) direction: String,
}

pub(super) fn parse_order_key(value: &str) -> Result<OrderKey, String> {
    let parts = value.split_whitespace().collect::<Vec<_>>();
    match parts.as_slice() {
        [key, direction] if matches!(direction.to_ascii_uppercase().as_str(), "ASC" | "DESC") => {
            Ok(OrderKey {
                key: key.to_ascii_lowercase(),
                direction: direction.to_ascii_uppercase(),
            })
        }
        [key] => Ok(OrderKey {
            key: key.to_ascii_lowercase(),
            direction: String::from("ASC"),
        }),
        _ => Err(format!(
            "ORDER BY declaration `{value}` must contain one key and optional ASC/DESC"
        )),
    }
}

pub(super) fn normalize_sql(sql: &str) -> String {
    sql.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
        .replace(" ,", ",")
        .replace(", ", ",")
        .replace("( ", "(")
        .replace(" )", ")")
}

/// The checker deliberately recognizes only SQL with a bound LIMIT and an
/// explicit bound cursor comparison.  This avoids treating bounded worker
/// batches and single-row lookups as page contracts; callers using another
/// pagination shape must add a narrow declaration and checker support.
pub(super) fn is_paginated_sql(sql: &str) -> bool {
    let normalized = normalize_sql(sql);
    normalized.contains("limit $")
        && (normalized.contains(" > $")
            || normalized.contains(" < $")
            || normalized.contains(") > (")
            || normalized.contains(") < ("))
}

pub(super) fn uuid_row_lookup_matches(sql: &str) -> bool {
    let normalized = normalize_sql(sql);
    normalized.contains("where cursor.id = $") || normalized.contains("where cursor_secret.id = $")
}

pub(super) fn parse_order_by(sql: &str) -> Result<Vec<OrderKey>, String> {
    let normalized = normalize_sql(sql);
    let Some(start) = top_level_clause_positions(&normalized, "order by ")
        .last()
        .copied()
    else {
        return Err(String::from("SQL query has no ORDER BY clause"));
    };
    let order = &normalized[start + "order by ".len()..];
    let end = [" limit ", " offset ", " fetch ", ";"]
        .iter()
        .filter_map(|marker| top_level_clause_positions(order, marker).first().copied())
        .min()
        .unwrap_or(order.len());
    let order = &order[..end];
    let values = order
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(parse_order_key)
        .collect::<Result<Vec<_>, _>>()?;
    if values.is_empty() {
        return Err(String::from("SQL ORDER BY clause has no keys"));
    }
    Ok(values)
}

fn top_level_clause_positions(sql: &str, clause: &str) -> Vec<usize> {
    let mut positions = Vec::new();
    let mut depth = 0_usize;
    let bytes = sql.as_bytes();
    let clause_bytes = clause.as_bytes();
    for index in 0..bytes.len() {
        match bytes[index] {
            b'(' => depth = depth.saturating_add(1),
            b')' => depth = depth.saturating_sub(1),
            _ => {}
        }
        if depth == 0 && bytes[index..].starts_with(clause_bytes) {
            positions.push(index);
        }
    }
    positions
}

pub(super) fn cursor_predicate_matches(
    sql: &str,
    keys: &[String],
    operator: &str,
    mode: &str,
) -> bool {
    let normalized = normalize_sql(sql);
    let tuple = keys.join(",");
    let tuple_pattern = format!("({tuple}) {operator} ");
    if normalized.contains(&tuple_pattern) {
        return true;
    }
    if keys.len() == 1 {
        return normalized.contains(&format!("{} {operator} ", keys[0]));
    }
    mode == "tuple"
        && keys.iter().all(|key| {
            normalized.contains(&format!("{key} {operator} "))
                || normalized.contains(&format!("{key} = "))
        })
}

pub(super) fn contains_schema_sql(sql: &str) -> bool {
    let words = sql
        .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
        .filter(|word| !word.is_empty())
        .map(str::to_ascii_uppercase)
        .collect::<Vec<_>>();
    words.windows(2).any(|pair| {
        matches!(
            (pair[0].as_str(), pair[1].as_str()),
            (
                "CREATE" | "ALTER" | "DROP",
                "TABLE" | "INDEX" | "SCHEMA" | "TYPE" | "POLICY" | "FUNCTION" | "TRIGGER" | "ROLE"
            ) | ("TRUNCATE", "TABLE")
        )
    })
}
