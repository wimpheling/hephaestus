//! Public gateway management projections and command contracts.

use gateway_domain::GatewayServiceConfig;
use release_domain::{ParameterName, ParameterValue};
use std::collections::BTreeMap;
use time::OffsetDateTime;
use uuid::Uuid;

/// A bounded cursor request for redacted gateway management projections.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GatewayPage {
    /// Maximum number of rows returned.
    pub limit: i64,
    /// Strictly older record id in the documented stable order.
    pub after: Option<Uuid>,
}

/// Safe gateway projection for management clients.
#[derive(Debug, Clone)]
pub struct GatewayManagementSummary {
    /// Durable gateway identity.
    pub id: Uuid,
    /// Owning project identity.
    pub project_id: Uuid,
    /// Owning repository identity.
    pub repository_id: Uuid,
    /// Stable declaration name.
    pub name: String,
    /// Current lifecycle.
    pub lifecycle: String,
    /// Exact current immutable revision, when installed.
    pub active_revision_id: Option<Uuid>,
    /// Latest declared HTTP service revision, which may still be pending
    /// readiness and therefore differ from the serving revision.
    pub desired_service_revision_id: Option<Uuid>,
    /// Latest lifecycle/configuration change time.
    pub updated_at: OffsetDateTime,
}

/// Safe immutable revision projection without parameters or secrets.
#[derive(Debug, Clone)]
pub struct GatewayManagementRevision {
    /// Immutable revision identity.
    pub id: Uuid,
    /// Source release identity.
    pub release_id: Option<Uuid>,
    /// Exact published agent identity that produced this revision.
    pub release_agent_id: Option<Uuid>,
    /// Supported handler contract.
    pub handler_contract: String,
    /// Typed loopback service declaration, when this is a persistent service.
    pub service: Option<GatewayServiceConfig>,
    /// Declared exposure policy.
    pub exposure: String,
    /// Symbolic declared secret slot names only.
    pub secret_slots: Vec<String>,
    /// Symbolic mailbox slots declared by the immutable source manifest.
    pub mailbox_slots: Vec<String>,
    /// Immutable creation time.
    pub created_at: OffsetDateTime,
    /// Immutable route intents in this revision.
    pub routes: Vec<GatewayManagementRoute>,
}

/// Safe route intent projection.
#[derive(Debug, Clone)]
pub struct GatewayManagementRoute {
    /// Durable route identity.
    pub id: Uuid,
    /// Canonical path below the reserved namespace.
    pub path: String,
    /// Bounded accepted methods.
    pub methods: Vec<String>,
    /// Whether this declared route is selectable.
    pub enabled: bool,
}

/// Value-free ingress audit projection.
#[derive(Debug, Clone)]
pub struct GatewayIngressSummary {
    /// Invocation identity.
    pub id: Uuid,
    /// Exact selected revision.
    pub gateway_revision_id: Uuid,
    /// Exact selected route.
    pub gateway_route_id: Uuid,
    /// Terminal or pending safe outcome.
    pub outcome: String,
    /// Acceptance time.
    pub accepted_at: OffsetDateTime,
    /// Terminal time, if the invocation completed.
    pub completed_at: Option<OffsetDateTime>,
}

/// Value-free projection of one immutable gateway mailbox binding and its
/// separately revocable grant.
#[derive(Debug, Clone)]
pub struct GatewayMailboxBindingSummary {
    /// Immutable binding identity.
    pub id: Uuid,
    /// Exact gateway revision that declared the slot.
    pub gateway_revision_id: Uuid,
    /// Exact target mailbox.
    pub mailbox_id: Uuid,
    /// Declared symbolic slot key.
    pub slot_key: String,
    /// Stable bound producer identity.
    pub producer_id: String,
    /// Separately revocable grant identity.
    pub grant_id: Uuid,
    /// `active` or `revoked`.
    pub grant_status: String,
    /// Immutable binding creation time.
    pub created_at: OffsetDateTime,
    /// Grant creation time.
    pub granted_at: OffsetDateTime,
    /// Grant revocation time, when revoked.
    pub revoked_at: Option<OffsetDateTime>,
}

/// Value-free correlation from a gateway invocation to one publication
/// settlement. Payload, headers, deduplication keys, and denial detail stay
/// outside management projections.
#[derive(Debug, Clone)]
pub struct GatewayMailboxPublicationSummary {
    /// Immutable publication audit identity.
    pub id: Uuid,
    /// Exact gateway invocation.
    pub invocation_id: Uuid,
    /// Revision selected for the invocation.
    pub gateway_revision_id: Uuid,
    /// Binding used when publication was accepted, if any.
    pub binding_id: Option<Uuid>,
    /// Grant used when publication was accepted, if any.
    pub grant_id: Option<Uuid>,
    /// Mailbox selected by the binding, if any.
    pub mailbox_id: Option<Uuid>,
    /// Accepted logical mailbox event, if any.
    pub event_id: Option<Uuid>,
    /// Declared slot selected by the guest.
    pub slot_key: String,
    /// Redacted `accepted`, `duplicate`, or `denied` result.
    pub outcome: String,
    /// Publication acceptance attempt time.
    pub accepted_at: OffsetDateTime,
    /// Publication settlement time.
    pub settled_at: OffsetDateTime,
    /// Immutable authorization snapshot selected for the invocation.
    pub authorization_snapshot_id: Option<Uuid>,
    /// Exact mailbox binding ordinal copied into that authorization snapshot.
    pub snapshot_binding_ordinal: Option<i32>,
    /// Current or terminal state of the accepted mailbox delivery.
    pub delivery_disposition: Option<String>,
    /// Number of logical delivery attempts so far.
    pub delivery_attempt_count: Option<i32>,
    /// Durable terminal time, when delivery reached a final disposition.
    pub delivery_terminal_at: Option<OffsetDateTime>,
    /// Most recent delivery-attempt identity, when one exists.
    pub delivery_attempt_id: Option<Uuid>,
    /// Run started by that delivery attempt, when one exists.
    pub run_id: Option<Uuid>,
    /// Current lifecycle state of that run.
    pub run_state: Option<String>,
    /// Final run outcome, when the run is terminal.
    pub run_outcome: Option<String>,
}

/// One explicitly selected inbound secret for a configured gateway revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewaySecretSelection {
    /// Declaration slot receiving this imported secret.
    pub slot_key: String,
    /// Project-owned import authority.
    pub import_id: Uuid,
    /// Exact immutable secret version.
    pub secret_version_id: Uuid,
    /// Declared route receiving the brokered header.
    pub route_path: String,
    /// Header populated by the broker.
    pub header_name: String,
}

/// Runtime values and secret selections for one immutable gateway revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigureGatewayRequest {
    /// Gateway whose declared revision is being configured.
    pub gateway_id: Uuid,
    /// Stateless gateways compare this with the active revision. Service
    /// gateways compare it with the latest desired revision, falling back to
    /// the active revision only when no desired candidate exists.
    pub expected_revision_id: Uuid,
    /// Typed values validated against the published agent schema.
    pub parameters: BTreeMap<ParameterName, ParameterValue>,
    /// Explicit inbound secret selections; no existing grants are copied.
    pub secret_selections: Vec<GatewaySecretSelection>,
}

/// Result of configuring one immutable gateway revision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConfigureGatewayResult {
    /// New immutable candidate revision, or the original result on replay.
    pub revision_id: Uuid,
}

/// Safe failures for the narrow gateway configuration operation.
#[derive(Debug, thiserror::Error)]
pub enum GatewayConfigureError {
    /// Caller lacks project/gateway/secret-import authority.
    #[error("gateway configuration is not authorized")]
    Denied,
    /// Selected gateway or revision is absent.
    #[error("gateway configuration target is unavailable")]
    NotFound,
    /// Typed values or secret selections violate the released declaration.
    #[error("gateway configuration request is invalid")]
    InvalidArgument,
    /// The expected active revision has changed.
    #[error("gateway configuration target is stale")]
    Stale,
    /// The same idempotency key was submitted with another payload.
    #[error("gateway configuration idempotency key conflicts")]
    Conflict,
    /// Database operation failed.
    #[error("gateway configuration is unavailable")]
    Persistence(#[from] sqlx::Error),
}

/// Safe failure category for management operations.
#[derive(Debug, thiserror::Error)]
pub enum GatewayManagementError {
    /// Caller lacks the exact project or gateway relation.
    #[error("gateway management is not authorized")]
    Denied,
    /// The selected gateway is hidden or absent.
    #[error("gateway is not found")]
    NotFound,
    /// Request values violate the bounded binding contract.
    #[error("gateway mailbox binding request is invalid")]
    InvalidArgument,
    /// A binding already exists or its grant is no longer active.
    #[error("gateway mailbox binding state conflicts")]
    Conflict,
    /// A storage failure prevented a safe result.
    #[error("gateway management is unavailable")]
    Unavailable,
    /// `PostgreSQL` persistence failed.
    #[error("gateway management persistence failed")]
    Persistence(#[from] sqlx::Error),
}
