//! One bounded part of the exact fixed catalog query inventory.
use serde_json::Value;
use sqlx::PgConnection;

pub(super) async fn read(
    connection: &mut PgConnection,
    name: &str,
    expected: &str,
) -> Result<(Value, String), sqlx::Error> {
    let (source, query) = match name {
        "enums" => (
            include_str!("../sql/enums.sql"),
            sqlx::query_as(include_str!("../sql/enums.sql")),
        ),
        "extension_members" => (
            include_str!("../sql/extension_members.sql"),
            sqlx::query_as(include_str!("../sql/extension_members.sql")),
        ),
        "extensions" => (
            include_str!("../sql/extensions.sql"),
            sqlx::query_as(include_str!("../sql/extensions.sql")),
        ),
        "foreign_tables" => (
            include_str!("../sql/foreign_tables.sql"),
            sqlx::query_as(include_str!("../sql/foreign_tables.sql")),
        ),
        "indexes" => (
            include_str!("../sql/indexes.sql"),
            sqlx::query_as(include_str!("../sql/indexes.sql")),
        ),
        "inheritance" => (
            include_str!("../sql/inheritance.sql"),
            sqlx::query_as(include_str!("../sql/inheritance.sql")),
        ),
        "initial_privileges" => (
            include_str!("../sql/initial_privileges.sql"),
            sqlx::query_as(include_str!("../sql/initial_privileges.sql")),
        ),
        "languages" => (
            include_str!("../sql/languages.sql"),
            sqlx::query_as(include_str!("../sql/languages.sql")),
        ),
        "memberships" => (
            include_str!("../sql/memberships.sql"),
            sqlx::query_as(include_str!("../sql/memberships.sql")),
        ),
        "namespaces" => (
            include_str!("../sql/namespaces.sql"),
            sqlx::query_as(include_str!("../sql/namespaces.sql")),
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
