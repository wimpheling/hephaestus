//! Private shared-Caddy reconciliation and bounded synchronous gateway dispatch.
//!
//! This crate deliberately knows no database schema or authorization relation.
//! Its ports let the gateway control plane supply only already-authorized active
//! routes while the local adapter owns the private Caddy administration boundary.

pub use gateway_domain::{
    DEFAULT_SERVICE_CONNECT_TIMEOUT, DEFAULT_SERVICE_MAX_CONNECTIONS, Exposure, GATEWAY_NAMESPACE,
    GatewayConfigRevision, GatewayDesiredConfiguration, GatewayEdgeError, GatewayExecutionTarget,
    GatewayExecutionTargetError, GatewayExecutionTargetResolver, GatewayInboundSecretResolver,
    GatewayInvocationOutcome, GatewayInvocationRecorder, GatewayLimits, GatewayMailboxPublisher,
    GatewayProvider, GatewayProviderResponse, GatewayReleaseResolver, GatewayRequest,
    GatewayRequestDispatcher, GatewayResponse, GatewayRouteBinding, GatewayRouteResolver,
    GatewayScheme, GatewayServiceArtifact, GatewayServiceArtifactKind,
    GatewayServiceAuthorityBudget, GatewayServiceClaimResolutionStore,
    GatewayServiceExpiredClaimRecovery, GatewayServiceFailure, GatewayServiceFailureCode,
    GatewayServiceFailureStore, GatewayServiceFailureStoreError, GatewayServiceIdentity,
    GatewayServiceInstanceKey, GatewayServiceInstanceLease, GatewayServiceInstancePage,
    GatewayServiceInstancePageResult, GatewayServiceInstanceState, GatewayServiceLaunch,
    GatewayServiceLaunchRequest, GatewayServiceLaunchResolver, GatewayServiceMaterializer,
    GatewayServiceOwnedTarget, GatewayServiceOwner, GatewayServiceOwnership,
    GatewayServiceOwnershipError, GatewayServiceRevisionTarget, GatewayServiceTarget,
    GatewayServiceTargetPage, GatewayServiceTargetPageResult, GatewayServiceTargetStore,
    InboundGatewaySecretRule, MAX_SERVICE_EXIT_VALUE, MAX_SERVICE_INSTANCE_PAGE_SIZE,
    MAX_SERVICE_OWNER_HOST_BYTES, MAX_SERVICE_OWNERSHIP_BATCH, MAX_SERVICE_OWNERSHIP_LEASE,
    MAX_SERVICE_TARGET_PAGE_SIZE, TrustedRequestMetadata, UNTRUSTED_FORWARDING_HEADERS,
    UiGatewayAdmission, UiGatewayAdmissionError, UiGatewayAdmissionProvider, UiGatewayAuthority,
    UiGatewayRequest, UiGatewayRequestKind, admission_failure_response, prepare_gateway_request,
    reject_ui_set_cookie, service_transport_spec, strip_ui_guest_headers, validate_ui_response,
};

mod caddy_config;
mod caddy_provider;
mod dispatcher;
mod dispatcher_execution;
mod runtime;
mod validation;

mod integration;
mod service_boot_recovery;
mod service_capacity;
mod service_cleanup;
mod service_cleanup_driver;
mod service_coordinator;
pub(crate) mod service_diagnostics;
mod service_handler;
mod service_http;
mod service_instance;
mod service_lease;
mod service_log_writer;
mod service_logs;
mod service_preparation;
mod service_probe;
mod service_registry;
mod service_supervisor;

#[cfg(test)]
mod tests;

pub use caddy_config::LocalCaddyConfigurationTemplate;
pub use caddy_provider::{CaddyAdministration, LocalCaddyGatewayProvider};
pub use dispatcher::{GatewayDispatcher, UiDispatchDisposition, UiDispatchResult};
pub use runtime::{
    GatewayRuntimeLauncher, GatewayRuntimeService, GatewayVmHandler, GatewayVmLauncher,
    PrivateHttpVmGatewayHandler,
};

#[cfg(test)]
pub(crate) use caddy_config::caddy_gateway_routes;
#[cfg(test)]
pub(crate) use validation::{empty_response, validate_mailbox_publication, validate_request};

pub use integration::caddy::LocalCaddyAdministration;
pub use service_boot_recovery::{
    GatewayServiceBootRecovery, GatewayServiceBootRecoveryContext, GatewayServiceBootRecoveryError,
    GatewayServiceBootRecoveryEvent, GatewayServiceBootRecoveryShutdown,
    GatewayServiceBootRecoveryUnresolved, MAX_SERVICE_BOOT_RECOVERY_CLEANUPS,
};
pub use service_capacity::{
    DEFAULT_SERVICE_DRAIN_TIMEOUT, DEFAULT_SERVICE_HEALTH_FAILURES,
    DEFAULT_SERVICE_HEALTH_INTERVAL, DEFAULT_SERVICE_MAX_REVISIONS_PER_GATEWAY,
    DEFAULT_SERVICE_MAX_STARTUPS, DEFAULT_SERVICE_REPLACEMENT_CAPACITY,
    DEFAULT_SERVICE_REQUEST_CAPACITY, DEFAULT_SERVICE_SERVING_CAPACITY, GatewayServiceCapacity,
    GatewayServiceCapacityError, GatewayServiceCapacitySnapshot, GatewayServiceCapacityToken,
    GatewayServiceSupervisorPolicy,
};
pub use service_cleanup::{GatewayServiceCleanup, GatewayServiceCleanupError};
pub use service_cleanup_driver::{
    GatewayServiceCleanupDriver, GatewayServiceCleanupDriverError,
    GatewayServiceCleanupDriverOutcome, GatewayServiceCleanupDriverPolicy,
    MAX_SERVICE_CLEANUP_DATABASE_WAIT,
};
pub use service_coordinator::{
    GatewayServiceCoordinator, GatewayServiceCoordinatorControl, GatewayServiceCoordinatorError,
    GatewayServiceCoordinatorFailure, GatewayServiceCoordinatorFailureReason,
    GatewayServiceCoordinatorStatus, GatewayServiceLogWriterConfig, GatewayServiceStartupIntent,
};
pub use service_diagnostics::{ServiceDiagnosticsSnapshot, ServiceLifecycleEvidence};
pub use service_handler::GatewayServiceHandler;
pub use service_http::{ServiceHttpPolicy, exchange as exchange_private_service_http};
pub use service_instance::{
    ServiceInstance, ServiceInstanceError, ServiceInstanceHandle, ServiceInstancePolicy,
    ServiceWorkerState, new_service_instance,
};
pub use service_lease::{
    GatewayServiceLeaseControl, GatewayServiceLeaseError, GatewayServiceLeaseLossReason,
    GatewayServiceLeaseMonitor, GatewayServiceLeasePolicy, GatewayServiceLeaseRetryReason,
    GatewayServiceLeaseRunResult, GatewayServiceLeaseStatus,
};
pub use service_log_writer::{
    DEFAULT_SERVICE_LOG_APPEND_TIMEOUT, DEFAULT_SERVICE_LOG_FINAL_FLUSH_TIMEOUT,
    DEFAULT_SERVICE_LOG_MAX_RETRY_INTERVAL, DEFAULT_SERVICE_LOG_RETRY_INTERVAL, ServiceLogWriter,
    ServiceLogWriterError, ServiceLogWriterFlush, ServiceLogWriterPolicy, ServiceLogWriterPoll,
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
    MAX_SERVICE_LOG_READ_PAGE_RECORDS, ServiceLogBufferHandle, ServiceLogBufferSnapshot,
    ServiceLogLoss, ServiceLogRecord,
};
pub use service_preparation::{
    PreparedGatewayService, ServicePreparation, ServicePreparationFailure,
    ServicePreparationFailureReason, ServicePreparationHandle, new_service_preparation,
};
pub use service_probe::{
    ServiceProbeError, ServiceProbePolicy, ServiceProbeSuccess, probe_private_service_http,
};
pub use service_registry::{
    GatewayServiceRegistry, GatewayServiceRegistryError, MAX_SERVICE_REGISTRY_CAPACITY,
    MAX_SERVICE_REQUEST_CAPACITY,
};
pub use service_supervisor::{
    GatewayServiceStartupHandle, GatewayServiceStartupRequest, GatewayServiceSupervisor,
    GatewayServiceSupervisorContext, GatewayServiceSupervisorError, GatewayServiceSupervisorEvent,
    GatewayServiceSupervisorJobStatus, GatewayServiceSupervisorShutdown,
    GatewayServiceSupervisorUnresolved,
};
