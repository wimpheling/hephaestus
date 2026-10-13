//! One bounded part of the exact fixed catalog query inventory.
use serde_json::Value;
use sqlx::PgConnection;

pub(super) async fn read(
    connection: &mut PgConnection,
    name: &str,
    expected: &str,
) -> Result<(Value, String), sqlx::Error> {
    let (source, query) = match name {
        "rules" => (
            include_str!("../sql/rules.sql"),
            sqlx::query_as(include_str!("../sql/rules.sql")),
        ),
        "security_labels" => (
            include_str!("../sql/security_labels.sql"),
            sqlx::query_as(include_str!("../sql/security_labels.sql")),
        ),
        "sequence_names" => (
            include_str!("../sql/sequence_names.sql"),
            sqlx::query_as(include_str!("../sql/sequence_names.sql")),
        ),
        "sequences" => (
            include_str!("../sql/sequences.sql"),
            sqlx::query_as(include_str!("../sql/sequences.sql")),
        ),
        "shared_dependencies" => (
            include_str!("../sql/shared_dependencies.sql"),
            sqlx::query_as(include_str!("../sql/shared_dependencies.sql")),
        ),
        "shared_descriptions" => (
            include_str!("../sql/shared_descriptions.sql"),
            sqlx::query_as(include_str!("../sql/shared_descriptions.sql")),
        ),
        "shared_security_labels" => (
            include_str!("../sql/shared_security_labels.sql"),
            sqlx::query_as(include_str!("../sql/shared_security_labels.sql")),
        ),
        "triggers" => (
            include_str!("../sql/triggers.sql"),
            sqlx::query_as(include_str!("../sql/triggers.sql")),
        ),
        "types" => (
            include_str!("../sql/types.sql"),
            sqlx::query_as(include_str!("../sql/types.sql")),
        ),
        "views" => (
            include_str!("../sql/views.sql"),
            sqlx::query_as(include_str!("../sql/views.sql")),
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
