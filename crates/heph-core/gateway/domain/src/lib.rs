//! Canonical, provider-neutral gateway declarations and bounded HTTP values.
//!
//! This crate deliberately contains no listener, provider, database, or VM
//! code. It is the exact shared vocabulary for the control plane and edge.

mod declaration;
mod identifiers;
mod ports;
mod root_contracts;
mod routing;
mod service_claim_resolution;
mod service_execution;
mod service_expired_claim_recovery;
mod service_failure;
mod service_instance_key;
mod service_launch;
mod service_logs;
mod service_ownership;
mod service_targets;
mod ui_gateway_admission;

pub use declaration::{
    GatewayDeclaration, GatewayError, GatewayMailboxPublicationSlot, GatewayServiceConfig,
    ServiceLogCaptureMode,
};
pub use identifiers::{
    GatewayId, GatewayInboundSecretResolver, GatewayName, GatewayReconciliationId,
    GatewayRevisionId, GatewayRouteId, GatewayTargetId, HTTP_HANDLER_CONTRACT_V1,
    HTTP_SERVICE_HANDLER_CONTRACT_V1, InboundGatewaySecretRule, MAX_BODY_BYTES,
    MAX_MAILBOX_PUBLICATION_SLOTS, MAX_ROUTES,
};
pub use ports::{
    GatewayInvocationOutcome, GatewayInvocationRecorder, GatewayMailboxPublisher,
    GatewayReleaseResolver,
};
pub use root_contracts::{
    GATEWAY_NAMESPACE, GatewayConfigRevision, GatewayDesiredConfiguration, GatewayEdgeError,
    GatewayLimits, GatewayProvider, GatewayProviderResponse, GatewayRequest,
    GatewayRequestDispatcher, GatewayResponse, GatewayRouteBinding, GatewayRouteResolver,
    GatewayScheme, TrustedRequestMetadata, UNTRUSTED_FORWARDING_HEADERS,
};
pub use routing::{
    Exposure, GatewayLifecycle, HttpMethod, RouteIntent, RoutePath, ServiceProbePath,
};
pub use service_claim_resolution::GatewayServiceClaimResolutionStore;
pub use service_execution::{
    GatewayExecutionTarget, GatewayExecutionTargetError, GatewayExecutionTargetResolver,
    GatewayServiceAuthorityBudget,
};
pub use service_expired_claim_recovery::GatewayServiceExpiredClaimRecovery;
pub use service_failure::{
    GatewayServiceFailure, GatewayServiceFailureCode, GatewayServiceFailureStore,
    GatewayServiceFailureStoreError, MAX_SERVICE_EXIT_VALUE,
};
pub use service_instance_key::GatewayServiceInstanceKey;
pub use service_launch::{
    DEFAULT_SERVICE_CONNECT_TIMEOUT, DEFAULT_SERVICE_MAX_CONNECTIONS, GatewayServiceArtifact,
    GatewayServiceArtifactKind, GatewayServiceIdentity, GatewayServiceLaunch,
    GatewayServiceLaunchRequest, GatewayServiceLaunchResolver, GatewayServiceMaterializer,
    service_transport_spec,
};
pub use service_logs::{
    GatewayServiceLogAppendBatch, GatewayServiceLogAppendOutcome, GatewayServiceLogMaintenance,
    GatewayServiceLogMaintenanceError, GatewayServiceLogMaintenancePolicy,
    GatewayServiceLogMaintenanceProjectPage, GatewayServiceLogMaintenanceProjectPageResult,
    GatewayServiceLogMaintenanceProjects, GatewayServiceLogMaintenanceReport,
    GatewayServiceLogProjectMetadata, GatewayServiceLogReadCursor, GatewayServiceLogReadMetadata,
    GatewayServiceLogReadPage, GatewayServiceLogReadRecord, GatewayServiceLogReadRequest,
    GatewayServiceLogReadScope, GatewayServiceLogStore, GatewayServiceLogStoreError,
    MAX_SERVICE_LOG_CHUNK_BYTES, MAX_SERVICE_LOG_INSTANCE_BYTES, MAX_SERVICE_LOG_INSTANCE_CHUNKS,
    MAX_SERVICE_LOG_MAINTENANCE_CHUNKS, MAX_SERVICE_LOG_MAINTENANCE_EPOCHS,
    MAX_SERVICE_LOG_MAINTENANCE_PROJECT_PAGE_SIZE, MAX_SERVICE_LOG_PROJECT_BYTES,
    MAX_SERVICE_LOG_PROJECT_CHUNKS, MAX_SERVICE_LOG_PROJECT_EPOCHS, MAX_SERVICE_LOG_QUEUE_BYTES,
    MAX_SERVICE_LOG_QUEUE_CHUNKS, MAX_SERVICE_LOG_READ_PAGE_BYTES,
    MAX_SERVICE_LOG_READ_PAGE_RECORDS, ServiceLogBufferSnapshot, ServiceLogLoss, ServiceLogRecord,
};
pub use service_ownership::{
    GatewayServiceInstanceLease, GatewayServiceInstanceState, GatewayServiceOwner,
    GatewayServiceOwnership, GatewayServiceOwnershipError, MAX_SERVICE_OWNER_HOST_BYTES,
    MAX_SERVICE_OWNERSHIP_BATCH, MAX_SERVICE_OWNERSHIP_LEASE,
};
pub use service_targets::{
    GatewayServiceInstancePage, GatewayServiceInstancePageResult, GatewayServiceOwnedTarget,
    GatewayServiceRevisionTarget, GatewayServiceTarget, GatewayServiceTargetPage,
    GatewayServiceTargetPageResult, GatewayServiceTargetStore, MAX_SERVICE_INSTANCE_PAGE_SIZE,
    MAX_SERVICE_TARGET_PAGE_SIZE,
};
pub use ui_gateway_admission::{
    UiGatewayAdmission, UiGatewayAdmissionError, UiGatewayAdmissionProvider, UiGatewayAuthority,
    UiGatewayRequest, UiGatewayRequestKind, admission_failure_response, prepare_gateway_request,
    reject_ui_set_cookie, strip_ui_guest_headers, validate_ui_response,
};

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
