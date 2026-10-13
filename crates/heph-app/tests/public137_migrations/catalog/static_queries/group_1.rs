//! One bounded part of the exact fixed catalog query inventory.
use serde_json::Value;
use sqlx::PgConnection;

pub(super) async fn read(
    connection: &mut PgConnection,
    name: &str,
    expected: &str,
) -> Result<(Value, String), sqlx::Error> {
    let (source, query) = match name {
        "__context" => (
            include_str!("../sql/context.sql"),
            sqlx::query_as(include_str!("../sql/context.sql")),
        ),
        "aggregates" => (
            include_str!("../sql/aggregates.sql"),
            sqlx::query_as(include_str!("../sql/aggregates.sql")),
        ),
        "collations" => (
            include_str!("../sql/collations.sql"),
            sqlx::query_as(include_str!("../sql/collations.sql")),
        ),
        "columns" => (
            include_str!("../sql/columns.sql"),
            sqlx::query_as(include_str!("../sql/columns.sql")),
        ),
        "constraints" => (
            include_str!("../sql/constraints.sql"),
            sqlx::query_as(include_str!("../sql/constraints.sql")),
        ),
        "database" => (
            include_str!("../sql/database.sql"),
            sqlx::query_as(include_str!("../sql/database.sql")),
        ),
        "default_acl" => (
            include_str!("../sql/default_acl.sql"),
            sqlx::query_as(include_str!("../sql/default_acl.sql")),
        ),
        "defaults" => (
            include_str!("../sql/defaults.sql"),
            sqlx::query_as(include_str!("../sql/defaults.sql")),
        ),
        "dependencies" => (
            include_str!("../sql/dependencies.sql"),
            sqlx::query_as(include_str!("../sql/dependencies.sql")),
        ),
        "descriptions" => (
            include_str!("../sql/descriptions.sql"),
            sqlx::query_as(include_str!("../sql/descriptions.sql")),
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
