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
        "release_agents" => Some("release_agents"),
        "releases" => Some("releases"),
        "release_capability_requirements" => Some("release_capability_requirements"),
        "agent_instances" => Some("agent_instances"),
        "agent_instance_state_volumes" => Some("agent_instance_state_volumes"),
        "agent_instance_revisions" => Some("agent_instance_revisions"),
        "release_command_inbox" => Some("release_command_inbox"),
        "agent_instance_events" => Some("agent_instance_events"),
        "outbox" => Some("outbox"),
        _ => None,
    }
}

fn known_constraint(value: &str) -> Option<&'static str> {
    match value {
        "agent_instances_state_volume_fk" => Some("agent_instances_state_volume_fk"),
        "agent_instances_active_revision_fk" => Some("agent_instances_active_revision_fk"),
        "agent_instances_pkey" => Some("agent_instances_pkey"),
        "agent_instances_project_id_name_key" => Some("agent_instances_project_id_name_key"),
        "agent_instance_state_volumes_pkey" => Some("agent_instance_state_volumes_pkey"),
        "agent_instance_revisions_pkey" => Some("agent_instance_revisions_pkey"),
        "release_command_inbox_pkey" => Some("release_command_inbox_pkey"),
        _ => None,
    }
}

fn observe(step: &'static str, error: &sqlx::Error) {
    let metadata = Metadata::from_error(error);
    tracing::error!(
        operation = "import_agent",
        step,
        category = metadata.category,
        sqlstate = metadata.sqlstate.as_deref().unwrap_or("redacted"),
        table = metadata.table.unwrap_or("redacted"),
        constraint = metadata.constraint.unwrap_or("redacted"),
        "database operation failed"
    );
}

pub(super) fn database(step: &'static str, error: sqlx::Error) -> super::ReleaseServiceError {
    observe(step, &error);
    super::ReleaseServiceError::Database(error)
}

pub(super) fn retained(
    step: &'static str,
    error: super::ReleaseServiceError,
) -> super::ReleaseServiceError {
    if let super::ReleaseServiceError::Database(source) = &error {
        observe(step, source);
    }
    error
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_fields_survive_without_accepting_unknown_identifiers() {
        let metadata = Metadata::database(
            Some("23514"),
            Some("agent_instances"),
            Some("agent_instances_state_volume_fk"),
        );
        assert_eq!(metadata.sqlstate.as_deref(), Some("23514"));
        assert_eq!(metadata.table, Some("agent_instances"));
        assert_eq!(metadata.constraint, Some("agent_instances_state_volume_fk"));
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
            database("begin", sqlx::Error::PoolTimedOut),
            super::super::ReleaseServiceError::Database(sqlx::Error::PoolTimedOut)
        ));
        assert!(matches!(
            retained("revision", super::super::ReleaseServiceError::Unavailable),
            super::super::ReleaseServiceError::Unavailable
        ));
    }
}
