//! One bounded part of the exact fixed catalog query inventory.
use serde_json::Value;
use sqlx::PgConnection;

pub(super) async fn read(
    connection: &mut PgConnection,
    name: &str,
    expected: &str,
) -> Result<(Value, String), sqlx::Error> {
    let (source, query) = match name {
        "operator_classes" => (
            include_str!("../sql/operator_classes.sql"),
            sqlx::query_as(include_str!("../sql/operator_classes.sql")),
        ),
        "operator_families" => (
            include_str!("../sql/operator_families.sql"),
            sqlx::query_as(include_str!("../sql/operator_families.sql")),
        ),
        "operators" => (
            include_str!("../sql/operators.sql"),
            sqlx::query_as(include_str!("../sql/operators.sql")),
        ),
        "partitioning" => (
            include_str!("../sql/partitioning.sql"),
            sqlx::query_as(include_str!("../sql/partitioning.sql")),
        ),
        "policies" => (
            include_str!("../sql/policies.sql"),
            sqlx::query_as(include_str!("../sql/policies.sql")),
        ),
        "ranges" => (
            include_str!("../sql/ranges.sql"),
            sqlx::query_as(include_str!("../sql/ranges.sql")),
        ),
        "relations" => (
            include_str!("../sql/relations.sql"),
            sqlx::query_as(include_str!("../sql/relations.sql")),
        ),
        "role_settings" => (
            include_str!("../sql/role_settings.sql"),
            sqlx::query_as(include_str!("../sql/role_settings.sql")),
        ),
        "roles" => (
            include_str!("../sql/roles.sql"),
            sqlx::query_as(include_str!("../sql/roles.sql")),
        ),
        "routines" => (
            include_str!("../sql/routines.sql"),
            sqlx::query_as(include_str!("../sql/routines.sql")),
        ),
        _ => return Err(sqlx::Error::Protocol("unknown fixed catalog query".into())),
    };
    if source.as_bytes() != expected.as_bytes() {
        return Err(sqlx::Error::Protocol(
            "fixed catalog query differs from sealed specification".into(),
        ));
    }
    let (json, text): (Value, String) = query.fetch_one(connection).await?;
    Ok((json, text))
}
