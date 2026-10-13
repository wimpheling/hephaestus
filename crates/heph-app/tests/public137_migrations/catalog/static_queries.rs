//! Fixed catalog SQL sources; every `SQLx` argument is a direct static include.

mod group_1;
mod group_2;
mod group_3;
mod group_4;

use serde_json::Value;
use sqlx::PgConnection;

pub(super) async fn read(
    connection: &mut PgConnection,
    name: &str,
    expected: &str,
) -> Result<(Value, String), sqlx::Error> {
    match name {
        "__context" | "aggregates" | "collations" | "columns" | "constraints" | "database"
        | "default_acl" | "defaults" | "dependencies" | "descriptions" => {
            group_1::read(connection, name, expected).await
        }
        "enums" | "extension_members" | "extensions" | "foreign_tables" | "indexes"
        | "inheritance" | "initial_privileges" | "languages" | "memberships" | "namespaces" => {
            group_2::read(connection, name, expected).await
        }
        "operator_classes" | "operator_families" | "operators" | "partitioning" | "policies"
        | "ranges" | "relations" | "role_settings" | "roles" | "routines" => {
            group_3::read(connection, name, expected).await
        }
        "rules"
        | "security_labels"
        | "sequence_names"
        | "sequences"
        | "shared_dependencies"
        | "shared_descriptions"
        | "shared_security_labels"
        | "triggers"
        | "types"
        | "views" => group_4::read(connection, name, expected).await,
        _ => Err(sqlx::Error::Protocol("unknown fixed catalog query".into())),
    }
}
