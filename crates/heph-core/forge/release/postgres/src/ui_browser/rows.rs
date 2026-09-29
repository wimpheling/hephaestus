use identity_domain::UserId;
use release_domain::UiInstallationId;
use release_domain::ui_browser::UiBrowserRoute;
use sqlx::FromRow;
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Debug)]
pub(super) struct EligibilityInput {
    pub(super) actor_id: UserId,
    pub(super) parent_session_id: identity_domain::BrowserSessionId,
    pub(super) installation_id: UiInstallationId,
    pub(super) generation_id: release_domain::UiInstallationGenerationId,
    pub(super) route: UiBrowserRoute,
}

#[derive(Debug)]
pub(super) struct IssueEligibility {
    pub(super) organization_id: Uuid,
    pub(super) parent_expires_at: OffsetDateTime,
    pub(super) presentation: String,
}

#[derive(Debug, FromRow)]
pub(super) struct ParentRow {
    pub(super) user_id: Uuid,
    pub(super) issued_at: OffsetDateTime,
    pub(super) expires_at: OffsetDateTime,
    pub(super) revoked_at: Option<OffsetDateTime>,
}

#[derive(Debug, FromRow)]
pub(super) struct InstallationDiscovery {
    pub(super) scope: String,
    pub(super) organization_id: Option<Uuid>,
    pub(super) project_id: Option<Uuid>,
    pub(super) repository_id: Option<Uuid>,
}

#[derive(Debug, FromRow)]
pub(super) struct InstallationRow {
    pub(super) installation_id: Uuid,
    pub(super) scope: String,
    pub(super) lifecycle: String,
    pub(super) organization_id: Option<Uuid>,
    pub(super) project_id: Option<Uuid>,
    pub(super) repository_id: Option<Uuid>,
    pub(super) current_generation_id: Uuid,
    pub(super) generation_id: Uuid,
    pub(super) release_id: Uuid,
    pub(super) ui_key: String,
    pub(super) ui_scope: String,
}

#[derive(Debug, FromRow)]
pub(super) struct SourceRow {
    pub(super) state: String,
    pub(super) scope: String,
    pub(super) route_base: String,
    pub(super) presentation: String,
    pub(super) content_kind: String,
    pub(super) source_project_id: Uuid,
    pub(super) source_repository_id: Uuid,
    pub(super) source_org: Uuid,
}

#[derive(Debug, FromRow)]
pub(super) struct BindingRow {
    pub(super) binding_kind: String,
    pub(super) binding_key: String,
    pub(super) release_id: Uuid,
    pub(super) ui_key: String,
    pub(super) method: String,
    pub(super) route: String,
    pub(super) release_agent_id: Uuid,
    pub(super) gateway_id: Uuid,
    pub(super) gateway_revision_id: Uuid,
    pub(super) gateway_name: String,
    pub(super) exposure: String,
}

#[derive(Debug, FromRow)]
pub(super) struct GatewayRow {
    pub(super) gateway_name: String,
    pub(super) lifecycle: String,
    pub(super) active_revision_id: Option<Uuid>,
    pub(super) revision_id: Uuid,
    pub(super) release_id: Uuid,
    pub(super) release_agent_id: Uuid,
    pub(super) handler_contract: String,
    pub(super) project_id: Uuid,
    pub(super) repository_id: Uuid,
    pub(super) exposure: String,
    pub(super) path: String,
    pub(super) enabled: bool,
    pub(super) methods: Vec<String>,
}

#[derive(Debug, FromRow)]
pub(super) struct ManagedRow {
    pub(super) gateway_name: String,
    pub(super) route: String,
    pub(super) release_agent_id: Uuid,
}

#[derive(Debug, FromRow)]
pub(super) struct ApiRow {
    pub(super) api_key: String,
    pub(super) gateway_name: String,
    pub(super) method: String,
    pub(super) route: String,
    pub(super) release_agent_id: Uuid,
}

#[derive(Debug, FromRow)]
pub(super) struct IssuedRow {
    pub(super) issued_at: OffsetDateTime,
    pub(super) expires_at: OffsetDateTime,
}
