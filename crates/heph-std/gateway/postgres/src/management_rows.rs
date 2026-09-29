//! `PostgreSQL` row projections and conversions for gateway management.

use super::{
    GatewayIngressSummary, GatewayMailboxBindingSummary, GatewayMailboxPublicationSummary,
    GatewayManagementError, GatewayManagementRoute, GatewayManagementSummary,
};
use gateway_domain::GatewayServiceConfig;
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(sqlx::FromRow)]
pub struct ConfigureRevisionRow {
    pub project_id: Uuid,
    pub repository_id: Uuid,
    pub active_revision_id: Option<Uuid>,
    pub desired_service_revision_id: Option<Uuid>,
    pub lifecycle: String,
    pub release_id: Option<Uuid>,
    pub release_agent_id: Option<Uuid>,
    pub release_agent_key: Option<String>,
    pub handler_contract: String,
    pub service_loopback_port: Option<i32>,
    pub service_readiness_path: Option<String>,
    pub service_health_path: Option<String>,
    pub service_log_capture_mode: String,
    pub exposure: String,
    pub secret_slots: Vec<String>,
    pub mailbox_slots: Vec<String>,
    pub parameter_schema: Option<serde_json::Value>,
    pub release_state: Option<String>,
}

#[derive(sqlx::FromRow)]
pub struct ConfigureRouteRow {
    pub path: String,
    pub methods: Vec<String>,
    pub enabled: bool,
}

#[derive(sqlx::FromRow)]
pub struct ConfigureCommandRow {
    pub gateway_id: Uuid,
    pub expected_revision_id: Uuid,
    pub payload_hash: Vec<u8>,
    pub actor_id: Uuid,
    pub result_revision_id: Option<Uuid>,
}

#[derive(sqlx::FromRow)]
pub struct GatewaySummaryRow {
    pub id: Uuid,
    pub project_id: Uuid,
    pub repository_id: Uuid,
    pub name: String,
    pub lifecycle: String,
    pub active_revision_id: Option<Uuid>,
    pub desired_service_revision_id: Option<Uuid>,
    pub updated_at: OffsetDateTime,
}
impl From<GatewaySummaryRow> for GatewayManagementSummary {
    fn from(row: GatewaySummaryRow) -> Self {
        Self {
            id: row.id,
            project_id: row.project_id,
            repository_id: row.repository_id,
            name: row.name,
            lifecycle: row.lifecycle,
            active_revision_id: row.active_revision_id,
            desired_service_revision_id: row.desired_service_revision_id,
            updated_at: row.updated_at,
        }
    }
}
#[derive(sqlx::FromRow)]
pub struct GatewayRevisionRow {
    pub id: Uuid,
    pub release_id: Option<Uuid>,
    pub release_agent_id: Option<Uuid>,
    pub handler_contract: String,
    pub service_loopback_port: Option<i32>,
    pub service_readiness_path: Option<String>,
    pub service_health_path: Option<String>,
    pub service_log_capture_mode: String,
    pub exposure: String,
    pub secret_slots: Vec<String>,
    pub mailbox_slots: Vec<String>,
    pub created_at: OffsetDateTime,
}

impl GatewayRevisionRow {
    pub fn service_config(&self) -> Result<Option<GatewayServiceConfig>, GatewayManagementError> {
        super::management_helpers::service_config_from_columns(
            self.service_loopback_port,
            self.service_readiness_path.clone(),
            self.service_health_path.clone(),
            &self.service_log_capture_mode,
        )
    }
}

#[derive(sqlx::FromRow)]
pub struct GatewayRouteRow {
    pub id: Uuid,
    pub path: String,
    pub methods: Vec<String>,
    pub enabled: bool,
}
impl From<GatewayRouteRow> for GatewayManagementRoute {
    fn from(row: GatewayRouteRow) -> Self {
        Self {
            id: row.id,
            path: row.path,
            methods: row.methods,
            enabled: row.enabled,
        }
    }
}
#[derive(sqlx::FromRow)]
pub struct GatewayIngressRow {
    pub id: Uuid,
    pub gateway_revision_id: Uuid,
    pub gateway_route_id: Uuid,
    pub outcome: String,
    pub accepted_at: OffsetDateTime,
    pub completed_at: Option<OffsetDateTime>,
}
impl From<GatewayIngressRow> for GatewayIngressSummary {
    fn from(row: GatewayIngressRow) -> Self {
        Self {
            id: row.id,
            gateway_revision_id: row.gateway_revision_id,
            gateway_route_id: row.gateway_route_id,
            outcome: row.outcome,
            accepted_at: row.accepted_at,
            completed_at: row.completed_at,
        }
    }
}
#[derive(sqlx::FromRow)]
pub struct GatewayMailboxBindingRow {
    pub id: Uuid,
    pub gateway_revision_id: Uuid,
    pub mailbox_id: Uuid,
    pub slot_key: String,
    pub producer_id: String,
    pub grant_id: Uuid,
    pub grant_status: String,
    pub created_at: OffsetDateTime,
    pub granted_at: OffsetDateTime,
    pub revoked_at: Option<OffsetDateTime>,
}
impl From<GatewayMailboxBindingRow> for GatewayMailboxBindingSummary {
    fn from(row: GatewayMailboxBindingRow) -> Self {
        Self {
            id: row.id,
            gateway_revision_id: row.gateway_revision_id,
            mailbox_id: row.mailbox_id,
            slot_key: row.slot_key,
            producer_id: row.producer_id,
            grant_id: row.grant_id,
            grant_status: row.grant_status,
            created_at: row.created_at,
            granted_at: row.granted_at,
            revoked_at: row.revoked_at,
        }
    }
}
#[derive(sqlx::FromRow)]
pub struct GatewayMailboxBindingTargetRow {
    pub gateway_revision_id: Uuid,
    pub instance_id: Uuid,
}

#[derive(sqlx::FromRow)]
pub struct GatewayMailboxBindingCommandRow {
    pub operation: String,
    pub gateway_revision_id: Uuid,
    pub slot_key: Option<String>,
    pub mailbox_id: Option<Uuid>,
    pub producer_id: Option<String>,
    pub target_binding_id: Option<Uuid>,
    pub payload_hash: Vec<u8>,
    pub actor_id: Uuid,
    pub result_binding_id: Option<Uuid>,
}

#[derive(sqlx::FromRow)]
pub struct GatewayMailboxPublicationManagementRow {
    pub id: Uuid,
    pub invocation_id: Uuid,
    pub gateway_revision_id: Uuid,
    pub binding_id: Option<Uuid>,
    pub grant_id: Option<Uuid>,
    pub mailbox_id: Option<Uuid>,
    pub event_id: Option<Uuid>,
    pub slot_key: String,
    pub outcome: String,
    pub accepted_at: OffsetDateTime,
    pub settled_at: OffsetDateTime,
    pub authorization_snapshot_id: Option<Uuid>,
    pub snapshot_binding_ordinal: Option<i32>,
    pub delivery_disposition: Option<String>,
    pub delivery_attempt_count: Option<i32>,
    pub delivery_terminal_at: Option<OffsetDateTime>,
    pub delivery_attempt_id: Option<Uuid>,
    pub run_id: Option<Uuid>,
    pub run_state: Option<String>,
    pub run_outcome: Option<String>,
}
impl From<GatewayMailboxPublicationManagementRow> for GatewayMailboxPublicationSummary {
    fn from(row: GatewayMailboxPublicationManagementRow) -> Self {
        Self {
            id: row.id,
            invocation_id: row.invocation_id,
            gateway_revision_id: row.gateway_revision_id,
            binding_id: row.binding_id,
            grant_id: row.grant_id,
            mailbox_id: row.mailbox_id,
            event_id: row.event_id,
            slot_key: row.slot_key,
            outcome: row.outcome,
            accepted_at: row.accepted_at,
            settled_at: row.settled_at,
            authorization_snapshot_id: row.authorization_snapshot_id,
            snapshot_binding_ordinal: row.snapshot_binding_ordinal,
            delivery_disposition: row.delivery_disposition,
            delivery_attempt_count: row.delivery_attempt_count,
            delivery_terminal_at: row.delivery_terminal_at,
            delivery_attempt_id: row.delivery_attempt_id,
            run_id: row.run_id,
            run_state: row.run_state,
            run_outcome: row.run_outcome,
        }
    }
}
