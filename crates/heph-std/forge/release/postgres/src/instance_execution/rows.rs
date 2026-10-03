use release_service::InstanceExecutionError;
use sqlx::FromRow;
use uuid::Uuid;

#[derive(FromRow)]
pub struct ActivationContext {
    pub original_import_key: Vec<u8>,
    pub release_agent: Uuid,
    pub first_mutex: i32,
    pub second_mutex: i32,
}

#[derive(FromRow)]
pub struct ActivationResult {
    pub activation: Uuid,
    pub instance: Uuid,
    pub revision: Uuid,
    pub version: i64,
}

pub fn database(error: &sqlx::Error) -> InstanceExecutionError {
    match error
        .as_database_error()
        .and_then(sqlx::error::DatabaseError::code)
        .as_deref()
    {
        Some("42501") => InstanceExecutionError::AuthorizationDenied,
        Some("P1140") => InstanceExecutionError::Unsupported,
        Some("P1141") => InstanceExecutionError::Closed,
        Some("P1142") => InstanceExecutionError::StaleInstance,
        Some("P1143" | "23505") => InstanceExecutionError::InputConflict,
        Some("P1144") => InstanceExecutionError::ConfigurationConflict,
        _ => InstanceExecutionError::OutcomeUncertain,
    }
}
