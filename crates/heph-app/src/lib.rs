//! Production composition root and supervised daemon lifecycle.
mod application;
mod event_adapter;
mod event_cursor;
pub mod rpc;
mod runtime_git_listener;
mod service_log_maintenance;
pub(crate) mod ui_audit;
mod ui_bootstrap;
mod ui_browser_content;
mod ui_context;
mod ui_origin_config;
mod ui_origin_wiring;
mod ui_repository_git;
pub use ui_origin_config::{UiOriginConfig, UiOriginConfigError};
#[path = "config_types.rs"]
mod config_types;
pub use config_types::{
    AppConfig, GatewayEdgeConfig, OciBuilderWorkerConfig, OidcConfig, RegistryConfig,
    RuntimePolicy, VmBackendConfig,
};
#[path = "composition/construction.rs"]
mod app_build;
pub use app_build::HephaestusApp;
#[path = "app_runtime.rs"]
mod app_runtime;
#[path = "config_validation.rs"]
mod config_validation;
#[path = "registry_adapters.rs"]
mod registry_adapters;
#[path = "composition/startup.rs"]
mod startup;
use registry_adapters::{
    InternalRegistryTokens, PostgresRegistryNotificationInbox, PostgresRegistryReconciliation,
    PostgresRegistryScopeAuthorizer, registry_caller_authentication, registry_reconciliation_loop,
};
#[path = "oci_workers.rs"]
mod oci_workers;
use app_runtime::{
    GatewayEdgeRuntime, LocalGatewayReleaseMaterializer, ProviderGatewayRuntimeLauncher,
};
use oci_workers::OciBuilderWorkers;
#[path = "gateway_dispatch.rs"]
mod gateway_dispatch;
use gateway_dispatch::{PrivateGatewayDispatcherState, gateway_limits, private_gateway_dispatch};
#[path = "gateway_service_state.rs"]
mod gateway_service_state;
use gateway_service_state::clone_service_supervisor_context;

#[path = "gateway_service_cleanup.rs"]
mod gateway_service_cleanup;

#[path = "gateway_reconciliation_loop.rs"]
mod gateway_reconciliation_loop;

#[path = "gateway_reconciliation.rs"]
mod gateway_reconciliation;
#[cfg(test)]
use gateway_reconciliation::gateway_reconciliation_loop;
use gateway_reconciliation::gateway_reconciliation_loop_with_context;
#[cfg(test)]
use gateway_reconciliation_loop::gateway_reconciliation_loop_with_boot;

#[path = "background_loops.rs"]
mod background_loops;
#[cfg(test)]
use background_loops::write_oci_manifest_if_dirty;
use background_loops::{
    OutboxWorker, mailbox_recovery_loop, oci_builder_loop, reap_failed_start,
    secret_revocation_loop, spawn_runtime_git_listener,
};

#[path = "update_completion.rs"]
mod update_completion;
#[cfg(test)]
use update_completion::deterministic_update_hook_run_id;
use update_completion::{UpdateRunCompletion, update_admission_reconciliation_loop};

#[path = "command_transport.rs"]
mod command_transport;
#[cfg(test)]
use command_transport::build_delivery_requires_redelivery;

#[path = "outbox_flush.rs"]
mod outbox_flush;
use outbox_flush::{FlushDiagnostics, FlushPublisher, flush_publisher, flush_until_quiescent};

#[path = "runtime_authority_manager.rs"]
mod runtime_authority_manager;
use runtime_authority_manager::PgRunAuthorityManager;
pub use runtime_authority_manager::RunEventKind;

#[path = "vm_spec_factory.rs"]
mod vm_spec_factory;
use vm_spec_factory::PgAgentVmSpecFactory;
#[cfg(test)]
use vm_spec_factory::StoredNetworkAccess;
#[cfg(test)]
use vm_spec_factory::{guest_environment, validate_runtime_policy};

#[path = "build_preparation.rs"]
mod build_preparation;
#[path = "fixture_vm.rs"]
mod fixture_vm;
pub use build_preparation::{AppError, EXPECTED_DATABASE_MIGRATION};
use build_preparation::{
    BuildPreparation, GATEWAY_SERVICE_REPLACEMENT_CAPACITY, GATEWAY_SERVICE_REQUEST_CAPACITY,
    GATEWAY_SERVICE_SERVING_CAPACITY, component, prepare as prepare_build,
};
#[cfg(test)]
mod tests;
mod ui_listener;
use command_transport::{build_loop, command_loop, mailbox_command_loop};

/// Test-only lifecycle synchronization hooks used by daemon integration tests.
#[cfg(feature = "test-fixtures")]
#[doc(hidden)]
pub mod test_hooks {
    pub use crate::application::commands::{
        CreateUpdateAdmissionBarrier, CreateUpdateAdmissionBarrierGuard,
        install_create_update_admission_barrier,
    };
}

use async_trait::async_trait;
use axum::{
    Router,
    routing::{any, get},
};
use build_orchestrator::{BuildExecutionError, BuildExecutor, BuildExecutorConfig};
use build_postgres::PgBuildRepository;
use builder_catalog_domain::OciImageReference;
use control_plane_postgres::launch::PgRunLaunchAuthorizer;
use control_plane_postgres::{
    ControlPlanePool, connect as connect_control_plane, connect_worker as connect_oci_worker,
};
use event_postgres::{ReleaseOutboxPublisher, ensure_release_jetstream_topology};
use forge_postgres::PgForgeRepository;
use forge_service::{
    ForgeNatsOutboxPublisher, GitStorage, ensure_build_consumer, ensure_forge_jetstream_topology,
};
use gateway_edge::{
    GatewayDispatcher, GatewayInboundSecretResolver, GatewayProvider, GatewayRequestDispatcher,
    GatewayRuntimeLauncher, GatewayRuntimeService, GatewayServiceArtifact,
    GatewayServiceArtifactKind, GatewayServiceBootRecovery, GatewayServiceBootRecoveryContext,
    GatewayServiceClaimResolutionStore, GatewayServiceCleanupDriverPolicy,
    GatewayServiceExpiredClaimRecovery, GatewayServiceHandler, GatewayServiceIdentity,
    GatewayServiceLogStore, GatewayServiceLogWriterConfig, GatewayServiceMaterializer,
    GatewayServiceOwner, GatewayServiceRegistry, GatewayServiceSupervisor,
    GatewayServiceSupervisorContext, GatewayServiceSupervisorPolicy, LocalCaddyAdministration,
    LocalCaddyConfigurationTemplate, LocalCaddyGatewayProvider, PrivateHttpVmGatewayHandler,
    ServiceLogWriterPolicy,
};
use gateway_postgres::{
    GatewayReleaseArtifact, GatewayReleaseArtifactKind, GatewayReleaseMaterializer,
    PostgresGatewayEdgeAuthority, PostgresGatewayExecutionTargetResolver,
    PostgresGatewayMailboxPublisher, PostgresGatewayReleaseResolver,
    PostgresGatewayServiceFailureStore, PostgresGatewayServiceLaunchResolver,
    PostgresGatewayServiceOwnership, PostgresGatewayServiceTargets,
};
use git_http::{
    CompositeGitAuthenticator, GitAuthenticator, GitHttpLimits, GitHttpService,
    OidcGitAuthenticator, PostgresGitAuthorizer, RuntimeGitHttpAuthenticator,
};
use heph_run::{
    CancelRun, CompositeRunCompletionObserver, Run, RunCompletionError, RunCompletionObserver,
    RunKind, RunOrchestrator, RunRepository, RunRuntimeArtifact, RunRuntimeArtifactKind,
};
use heph_secret::EphemeralSecretConfig;
use identity_application::BrowserSessionStore;
use identity_oidc::OidcVerifier;
use identity_postgres::{PostgresBrowserSessionStore, PostgresIdentityStore};
use jsonwebtoken::{Algorithm, DecodingKey};
use mailbox_dispatch::{
    MailboxCommandHandler, MailboxDispatchStore, MailboxOutboxPublisher, MailboxRunCompletion,
    MailboxRunResources, NatsMailboxCommandHandler, ensure_mailbox_jetstream_topology,
};
use mailbox_postgres::PostgresMailboxRepository;
use oci_builder_postgres::{PgOciImageProductionJobStore, PgRepositoryOciImagePublicationStore};
use oci_builder_runtime_local::{
    ForgeZotOciPublisher, LocalOciRuntime, LocalOciRuntimeConfig, VmOciOperation,
    VmOciOperationConfig, VmPublishedOciEngine,
};
use oci_builder_worker::{
    MaterializedRoot, OciImageProductionWorker, OciWorkerError, RegistryPublisherTokenIssuer,
    RootfsMaterializationWorker,
};
use registry_domain::{PolicyVersion, RegistryNamespace, SupplyChainPolicy};
use registry_http::{RegistryAuthorizationError, RegistryTokenHttpService};
use registry_notification::{NotificationAction, NotificationObservation};
use registry_notification_http::{
    InboxDisposition, RegistryInboxError, RegistryNotificationHttpService,
    RegistryNotificationInbox,
};
use registry_postgres::{
    NewRegistryNotification, NotificationCompletion as PgNotificationCompletion, PgRegistryStore,
    RegistryNotificationAction, RegistryNotificationTarget,
};
use registry_publisher::{ControlledOciPublisher, PublisherConfiguration, SystemCommandRunner};
use registry_reconciler::{
    ClaimedNotification, NotificationCompletion, NotificationInbox, PublicationIntents,
    ReconciliationAction, ReconciliationActionExecutor, ReconciliationPortError,
    RegistryReconciler,
};
use registry_token::{
    AuthorizationDecision as RegistryAuthorizationDecision, RegistryAction, RepositoryActions,
    RepositoryName, ScopeRequest, TokenSubject, UnixTimestamp,
};
use registry_zot::{RegistryPullTokenProvider, ZotClientConfig, ZotClientError, ZotHttpRegistry};
use release_artifact_store::LocalArtifactStore;
use release_domain::BuildRequestId;
use release_postgres::{
    PgUiBrowserServingStore, PgUiBrowserSessionStore, PgUiGenerationHostResolver,
    PgUiRequestAuditRepository, ReleaseService, ReleaseServiceError,
};
use release_service::{
    UiBrowserRepositoryGitAuthorization, UiBrowserSessionStore, UiGenerationHostResolver,
};
use review_postgres::PostgresReviewRepository;
use review_service::{NatsControlHandler, ReviewControlService, ReviewOutboxPublisher};
use run_orchestrator::{NatsCommandHandler, ensure_jetstream_topology};
use run_postgres::PgRunRepository;
use run_runtime_local::{
    GatewayServiceIdentity as LocalGatewayServiceIdentity, LocalGatewayReleaseRuntime,
    LocalRunRuntimeConfig, LocalRunRuntimeManager,
};
use runtime_authority::{GatewayRuntimeAuthorityIssuer, RuntimeHandoffStore};
use runtime_authority_postgres::PgGatewayRuntimeAuthorityIssuer;
use runtime_handoff_local::EncryptedFileHandoffStore;
use runtime_types::{CommandId, RunId};
use secret_application::BrokerAdapter;
use secret_broker::{BrokerExecutor, BrokerServer};
use secret_postgres::SecretService;
use secret_store::LocalKeyProvider;
use serde::Deserialize;
use service_log_maintenance::GatewayServiceLogMaintenanceScheduler;
type PgPool = ControlPlanePool;
use heph_runtime::{
    GuestCommand, NetworkMode, RootFilesystem, StopMode, VmError, VmEvent, VmExit, VmId,
    VmInstance, VmMetric, VmMount, VmProvider, VmResources, VmSpec,
};
use std::{
    collections::{BTreeMap, HashMap},
    future::Future,
    net::SocketAddr,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    sync::{
        Arc, Mutex as StdMutex, RwLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use time::OffsetDateTime;
use tokio::{
    sync::{Mutex, Semaphore},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use vm_libkrun::LibkrunConfig;
use volume_local::{LocalVolumeConfig, LocalVolumeStore};
use workspace_local::{LocalWorkspaceConfig, LocalWorkspaceManager};

fn canonicalize_release_artifact_root(path: PathBuf) -> Result<PathBuf, AppError> {
    std::fs::canonicalize(path).map_err(component("release artifact store"))
}

impl HephaestusApp {
    /// Validates configuration and constructs every production dependency.
    ///
    /// This does not bind listeners or spawn background tasks.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid configuration, incompatible migrations, or
    /// an unavailable required dependency.
    pub async fn build(config: AppConfig) -> Result<Self, AppError> {
        app_build::build(config).await
    }

    /// Binds HTTP, establishes durable NATS topology, and starts supervised
    /// background workers.
    ///
    /// This returns only after HTTP, command consumption, and outbox
    /// publication have crossed their readiness barriers.
    ///
    /// # Errors
    ///
    /// Returns an error when startup or readiness fails. Already-started tasks
    /// are cancelled and reaped before the error is returned.
    pub async fn start(self) -> Result<RunningHephaestus, AppError> {
        // Keep the large startup state machine off the caller's stack.
        Box::pin(self.start_inner()).await
    }

    async fn start_inner(self) -> Result<RunningHephaestus, AppError> {
        startup::start(self).await
    }
}

#[path = "composition/running.rs"]
mod running;
pub use running::RunningHephaestus;

#[cfg(test)]
mod gateway_recovery_tests;
