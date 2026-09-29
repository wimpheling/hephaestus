//! `PostgreSQL` adapter for static UI installation and lifecycle commands.

use release_service::UiInstallationError;
use uuid::Uuid;

#[derive(Debug, sqlx::FromRow)]
struct OwnerRow {
    #[sqlx(rename = "project_id")]
    project: Option<Uuid>,
    #[sqlx(rename = "organization_id")]
    organization: Uuid,
    #[sqlx(rename = "repository_id")]
    repository: Option<Uuid>,
}

#[derive(Debug, sqlx::FromRow)]
struct InstallationOwnerRow {
    #[sqlx(rename = "project_id")]
    project: Option<Uuid>,
    #[sqlx(rename = "organization_id")]
    organization: Uuid,
    #[sqlx(rename = "repository_id")]
    repository: Option<Uuid>,
    scope: String,
    ui_key: String,
}

#[derive(Debug, sqlx::FromRow)]
struct LockedInstallationRow {
    #[sqlx(rename = "project_id")]
    project: Option<Uuid>,
    #[sqlx(rename = "organization_id")]
    organization: Uuid,
    #[sqlx(rename = "repository_id")]
    repository: Option<Uuid>,
    scope: String,
    ui_key: String,
    lifecycle: String,
    current_generation_id: Uuid,
    current_generation_no: i64,
}

#[derive(Debug, sqlx::FromRow)]
struct ExistingInstallCommand {
    command_key: Vec<u8>,
    input_hash: Vec<u8>,
    installation_id: Uuid,
    result_generation_id: Uuid,
    result_lifecycle: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AttemptError {
    Public(UiInstallationError),
    RetryLedgerRace,
}

impl From<UiInstallationError> for AttemptError {
    fn from(error: UiInstallationError) -> Self {
        Self::Public(error)
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UiBindingResolutionError {
    Invalid,
    Persistence,
}
#[derive(Debug, sqlx::FromRow)]
struct GatewayRouteCandidate {
    gateway_id: Uuid,
    gateway_revision_id: Uuid,
    gateway_name: String,
    handler_contract: String,
    route_path: String,
    route_methods: Vec<String>,
}

#[derive(Debug, Clone)]
struct ResolvedUiBinding {
    binding_kind: &'static str,
    binding_key: String,
    gateway_id: Uuid,
    gateway_revision_id: Uuid,
    release_agent_id: Uuid,
    gateway_name: String,
    method: String,
    route: String,
}
mod bindings;
mod commands;
mod gateway;
mod generation;
mod install_transaction;
mod ledger;
mod lifecycle;
mod owner;
mod persistence;
