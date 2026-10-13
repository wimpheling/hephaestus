//! Error metadata projection for bounded operational diagnostics.

#[derive(Debug, PartialEq, Eq)]
struct Metadata {
    category: &'static str,
    sqlstate: Option<String>,
    table: Option<&'static str>,
    constraint: Option<&'static str>,
}

impl Metadata {
    fn from_error(error: &sqlx::Error) -> Self {
        error.as_database_error().map_or_else(
            || Self {
                category: "driver",
                sqlstate: None,
                table: None,
                constraint: None,
            },
            |database| {
                Self::database(
                    database.code().as_deref(),
                    database.table(),
                    database.constraint(),
                )
            },
        )
    }

    fn database(code: Option<&str>, table: Option<&str>, constraint: Option<&str>) -> Self {
        Self {
            category: "database",
            sqlstate: code
                .filter(|value| {
                    value.len() == 5
                        && value
                            .bytes()
                            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
                })
                .map(str::to_owned),
            table: table.and_then(known_table),
            constraint: constraint.and_then(known_constraint),
        }
    }
}

// Match exact committed DDL identifiers; never echo an unrecognized server field.
fn known_table(value: &str) -> Option<&'static str> {
    match value {
        "organizations" => Some("organizations"),
        "projects" => Some("projects"),
        "secrets" => Some("secrets"),
        "secret_versions" => Some("secret_versions"),
        "secret_command_inbox" => Some("secret_command_inbox"),
        "secret_audit_events" => Some("secret_audit_events"),
        _ => None,
    }
}

fn known_constraint(value: &str) -> Option<&'static str> {
    match value {
        "secrets_active_version_fk" => Some("secrets_active_version_fk"),
        "secrets_pkey" => Some("secrets_pkey"),
        "secret_versions_pkey" => Some("secret_versions_pkey"),
        "secret_versions_secret_id_sequence_key" => Some("secret_versions_secret_id_sequence_key"),
        "secret_command_inbox_pkey" => Some("secret_command_inbox_pkey"),
        _ => None,
    }
}

fn observe(step: &'static str, error: &sqlx::Error) {
    let metadata = Metadata::from_error(error);
    tracing::error!(
        operation = "secret_command",
        step,
        category = metadata.category,
        sqlstate = metadata.sqlstate.as_deref().unwrap_or("redacted"),
        table = metadata.table.unwrap_or("redacted"),
        constraint = metadata.constraint.unwrap_or("redacted"),
        "database operation failed"
    );
}

pub(super) fn persistence(step: &'static str, error: &sqlx::Error) -> super::SecretServiceError {
    observe(step, error);
    super::SecretServiceError::Persistence
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_fields_survive_without_accepting_unknown_identifiers() {
        let metadata = Metadata::database(
            Some("23514"),
            Some("secrets"),
            Some("secrets_active_version_fk"),
        );
        assert_eq!(metadata.sqlstate.as_deref(), Some("23514"));
        assert_eq!(metadata.table, Some("secrets"));
        assert_eq!(metadata.constraint, Some("secrets_active_version_fk"));
        let hostile = Metadata::database(
            Some("23514"),
            Some("secret-value"),
            Some("private tenant constraint"),
        );
        assert_eq!(hostile.sqlstate, metadata.sqlstate);
        assert_eq!(hostile.table, None);
        assert_eq!(hostile.constraint, None);
    }

    #[test]
    fn malformed_sqlstate_is_redacted() {
        for value in ["23514 secret", "2351", "235140", "ab123", "23é14", "23\n14"] {
            assert_eq!(Metadata::database(Some(value), None, None).sqlstate, None);
        }
        assert_eq!(Metadata::database(None, None, None).sqlstate, None);
    }

    #[test]
    fn nondatabase_error_has_no_database_metadata() {
        let metadata = Metadata::from_error(&sqlx::Error::PoolTimedOut);
        assert_eq!(
            metadata,
            Metadata {
                category: "driver",
                sqlstate: None,
                table: None,
                constraint: None
            }
        );
        assert!(matches!(
            persistence("begin", &sqlx::Error::PoolTimedOut),
            super::super::SecretServiceError::Persistence
        ));
    }
}
