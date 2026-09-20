//! Production composition root and supervised daemon lifecycle.

mod application;
mod event_adapter;
mod event_cursor;
pub mod rpc;
mod service_log_maintenance;

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
    body::Body,
    extract::{ConnectInfo, State},
    http::Request,
    response::Response,
    routing::{any, get},
};
use build_orchestrator::{BuildExecutionError, BuildExecutor, BuildExecutorConfig};
use build_postgres::PgBuildRepository;
use builder_catalog_domain::OciImageReference;
use bytes::Bytes;
use capability_domain::{
    RuntimeCredentialGeneration, RuntimeInvocation, RuntimeSessionId, RuntimeSessionIdentity,
};
use control_plane_postgres::launch::PgRunLaunchAuthorizer;
use control_plane_postgres::{
    ControlPlanePool, connect as connect_control_plane, connect_app as connect_application,
    connect_worker as connect_oci_worker, is_update_hook_run, load_vm_launch_contract,
    pending_update_admissions, recoverable_update_hook_run_ids,
};
use event_postgres::{ReleaseOutboxPublisher, ensure_release_jetstream_topology};
use forge_postgres::PgForgeRepository;
use forge_service::{
    BUILD_REQUESTED_SUBJECT, BUILD_RETRY_REQUESTED_SUBJECT, BUILD_VERIFY_REQUESTED_SUBJECT,
    ForgeNatsOutboxPublisher, GitStorage, ensure_build_consumer, ensure_forge_jetstream_topology,
};
use futures_util::StreamExt;
use gateway_edge::{
    GatewayDispatcher, GatewayInboundSecretResolver, GatewayLimits, GatewayProvider,
    GatewayRequest, GatewayRequestDispatcher, GatewayRuntimeLauncher, GatewayRuntimeService,
    GatewayScheme, GatewayServiceArtifact, GatewayServiceArtifactKind, GatewayServiceBootRecovery,
    GatewayServiceBootRecoveryContext, GatewayServiceClaimResolutionStore,
    GatewayServiceCleanupDriverPolicy, GatewayServiceExpiredClaimRecovery, GatewayServiceHandler,
    GatewayServiceIdentity, GatewayServiceLogStore, GatewayServiceLogWriterConfig,
    GatewayServiceMaterializer, GatewayServiceOwnedTarget, GatewayServiceOwner,
    GatewayServiceRegistry, GatewayServiceStartupIntent, GatewayServiceStartupRequest,
    GatewayServiceSupervisor, GatewayServiceSupervisorContext, GatewayServiceSupervisorJobStatus,
    GatewayServiceSupervisorPolicy, GatewayServiceTargetPage, GatewayServiceTargetPageResult,
    LocalCaddyAdministration, LocalCaddyConfigurationTemplate, LocalCaddyGatewayProvider,
    PrivateHttpVmGatewayHandler, ServiceLogWriterPolicy, TrustedRequestMetadata,
    UNTRUSTED_FORWARDING_HEADERS,
};
use gateway_postgres::{
    GatewayReleaseArtifact, GatewayReleaseArtifactKind, GatewayReleaseMaterializer,
    PostgresGatewayEdgeAuthority, PostgresGatewayExecutionTargetResolver,
    PostgresGatewayMailboxPublisher, PostgresGatewayReleaseResolver,
    PostgresGatewayServiceFailureStore, PostgresGatewayServiceLaunchResolver,
    PostgresGatewayServiceLogStore, PostgresGatewayServiceOwnership, PostgresGatewayServiceTargets,
};
use git_http::{
    CompositeGitAuthenticator, GitAuthenticator, GitHttpLimits, GitHttpService,
    OidcGitAuthenticator, PostgresGitAuthorizer, RuntimeGitHttpAuthenticator,
};
use identity_application::IdempotentIdentityResolver;
use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
use identity_oidc::OidcVerifier;
use identity_postgres::PostgresIdentityStore;
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
use registry_http::{
    RegistryAuthorizationError, RegistryScopeAuthorizer, RegistryTokenHttpService,
};
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
    ClaimedNotification, NotificationCompletion, NotificationInbox, ObservedTarget,
    PublicationIntents, ReconciliationAction, ReconciliationActionExecutor,
    ReconciliationPortError, RegistryReconciler,
};
use registry_token::{
    AuthorizationDecision as RegistryAuthorizationDecision, IssuedToken, RegistryAction,
    RepositoryActions, RepositoryName, ScopeRequest, TokenSubject, UnixTimestamp,
};
use registry_zot::{RegistryPullTokenProvider, ZotClientConfig, ZotClientError, ZotHttpRegistry};
use release_artifact_store::LocalArtifactStore;
use release_domain::{BuildRequestId, ReleaseCommandKey};
use release_postgres::{ReleaseService, ReleaseServiceError};
use release_service::BeginUpdateHook;
use review_domain::CONTROL_EXECUTE_SUBJECT;
use review_postgres::{GitRepositoryLocator, PostgresReviewRepository};
use review_service::{NatsControlHandler, ReviewControlService, ReviewOutboxPublisher};
use run_domain::{CancelRun, Run, RunKind};
use run_orchestrator::{
    CompositeRunCompletionObserver, NatsCommandHandler, PreparedRunAuthority, RunAuthorityError,
    RunAuthorityManager, RunCompletionError, RunCompletionObserver, RunOrchestrator, RunRepository,
    RunRuntimeArtifact, RunRuntimeArtifactKind, RunSecretManager, VmSpecFactory,
    ensure_jetstream_topology,
};
use run_postgres::PgRunRepository;
use run_runtime_local::{
    GatewayServiceIdentity as LocalGatewayServiceIdentity, LocalGatewayReleaseRuntime,
    LocalRunRuntimeConfig, LocalRunRuntimeManager,
};
use runtime_authority::{
    GatewayRuntimeAuthorityIssuer, RuntimeHandoffStore, RuntimeSessionIssuer,
    RuntimeSessionRepository,
};
use runtime_authority_postgres::{PgGatewayRuntimeAuthorityIssuer, PgRuntimeSessionRepository};
use runtime_git_authority::{RuntimeGitAuthorityError, RuntimeGitCredentialIssuer};
use runtime_git_authority_postgres::PgRuntimeGitCredentialRepository;
use runtime_handoff_local::{EncryptedFileHandoffStore, EncryptedFileRuntimeGitHandoffStore};
use runtime_types::{CommandId, RunId};
use secret_application::BrokerAdapter;
use secret_broker::{BrokerExecutor, BrokerServer, ServiceBrokerExecutor};
use secret_postgres::initialize_manager;
use secret_postgres::{GatewayIngressSecretResolver, SecretRuntimeService, SecretService};
use secret_runtime::EphemeralSecretConfig;
use secret_store::{EncryptedStore, LocalKeyProvider};
use serde::Deserialize;
use service_log_maintenance::GatewayServiceLogMaintenanceScheduler;
use sha2::{Digest, Sha256};
type PgPool = ControlPlanePool;
use std::{
    collections::{BTreeMap, HashMap},
    future::Future,
    net::SocketAddr,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    pin::Pin,
    sync::{
        Arc, Mutex as StdMutex, RwLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use time::OffsetDateTime;
use tokio::{
    sync::{Mutex, Semaphore, broadcast, oneshot, watch},
    task::{JoinHandle, JoinSet},
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use vm_fake::FakeProvider;
use vm_libkrun::{LibkrunConfig, LibkrunProvider};
use vm_trait::{
    GuestCommand, NetworkMode, RootFilesystem, StopMode, VmError, VmEvent, VmExit, VmId,
    VmInstance, VmMetric, VmMount, VmProvider, VmResources, VmSpec,
};
use volume_local::{LocalVolumeConfig, LocalVolumeStore};
use volume_postgres::PostgresVolumeMetadataRepository;
use workspace_local::{LocalWorkspaceConfig, LocalWorkspaceManager};
use workspace_postgres::PgWorkspaceMetadataRepository;

/// Ordered database migration expected by this application version.
pub const EXPECTED_DATABASE_MIGRATION: i64 = 86;

const GATEWAY_SERVICE_SERVING_CAPACITY: usize = 8;
const GATEWAY_SERVICE_REPLACEMENT_CAPACITY: usize = 2;
const GATEWAY_SERVICE_REQUEST_CAPACITY: usize = 16;

/// OIDC issuer configuration used for bearer-token authentication.
#[derive(Clone)]
pub struct OidcConfig {
    /// Trusted issuer URL.
    pub issuer: String,
    /// Required token audience.
    pub audience: String,
    /// Expected JWT signature algorithm.
    pub algorithm: Algorithm,
    /// Trusted decoding key resolved from the issuer's JWKS.
    pub decoding_key: DecodingKey,
}

/// Forge-owned registry token-service configuration.
#[derive(Clone)]
pub struct RegistryConfig {
    /// RS256 token issuer whose private key remains in the Hephaestus process.
    pub token_issuer: Arc<registry_token::RegistryTokenIssuer>,
    /// Authenticator for Zot's private best-effort event callback.
    pub notification_callback: registry_notification::CallbackCredential,
    /// Fixed private Zot endpoint used only for authoritative digest reads.
    pub zot: ZotClientConfig,
    /// Lease applied to durable notification inbox claims.
    pub reconciliation_lease: Duration,
    /// Interval for both inbox draining and missed-event full reconciliation.
    pub reconciliation_interval: Duration,
}

/// Optional shared-Caddy gateway edge configuration.
///
/// When absent, the daemon does not create a gateway listener or attempt to
/// administer Caddy. This preserves the explicit operator boundary while
/// allowing deployments that do not expose repository gateways.
#[derive(Clone)]
pub struct GatewayEdgeConfig {
    /// Private loopback Caddy administration endpoint.
    pub caddy_admin_url: String,
    /// Complete operator-owned shared-Caddy JSON baseline.
    pub caddy_configuration_template: Vec<u8>,
    /// Existing shared-Caddy server containing the dedicated gateway subroute.
    pub caddy_server_name: String,
    /// Private loopback HTTP listener used only by the local Caddy process.
    pub dispatcher_listen: SocketAddr,
    /// Canonical public authority recorded as trusted gateway metadata.
    pub public_authority: String,
}

/// Configured VM backend.
#[derive(Clone)]
pub enum VmBackendConfig {
    /// Deterministic development and test provider.
    Fake,
    /// Deterministic local E2E guest which edits and finalizes its workspace.
    FixtureResult,
    /// Explicit provider injection for hardware-independent end-to-end tests.
    Custom(Arc<dyn VmProvider>),
    /// Production libkrun provider.
    Libkrun(Box<LibkrunConfig>),
}

/// Current platform ceiling applied again immediately before every guest boot.
#[derive(Clone, Debug)]
pub struct RuntimePolicy {
    /// Operator-defined policy revision recorded on launched VMs.
    pub version: String,
    /// Largest permitted virtual CPU allocation.
    pub max_vcpus: u8,
    /// Largest permitted memory allocation, in mebibytes.
    pub max_memory_mib: u32,
    /// Whether guests may use only the semantic secret broker transport.
    pub allow_broker_only: bool,
    /// Whether guests may receive general outbound user-mode networking.
    pub allow_egress: bool,
}

/// Optional single-node OCI preparation and rootfs materialization workers.
#[derive(Clone)]
pub struct OciBuilderWorkerConfig {
    /// Administrator-owned local Git/OCI/scanner runtime.
    pub runtime: LocalOciRuntimeConfig,
    /// Fixed Zot publication boundary and trusted OCI client binaries.
    pub publisher: PublisherConfiguration,
    /// Immutable policy revision recorded with every repository publication.
    pub publication_policy_version: PolicyVersion,
    /// Required evidence policy for repository builder images.
    pub publication_policy: SupplyChainPolicy,
    /// Exact operational root reference for the isolated Buildah VM.
    pub builder_vm_image: OciImageReference,
    /// Exact operational root reference for the independent verifier VM.
    pub verifier_vm_image: OciImageReference,
    /// Private verifier evidence/export root.
    pub verification_root: PathBuf,
    /// Private, per-operation Buildah storage root mounted only into builders.
    pub scratch_root: PathBuf,
    /// Absolute trusted formatter for per-operation ext4 scratch disks.
    pub mkfs_ext4: PathBuf,
    /// Fixed resources for both one-shot operation VMs.
    pub vm_resources: VmResources,
    /// Whether GCP Cooking may collect bounded informational workload timings.
    pub workload_phase_timing: bool,
    /// Stable identity for durable OCI preparation claims.
    pub preparation_worker_name: String,
    /// Stable daemon-local identity for rootfs materialization claims.
    pub materialization_worker_name: String,
    /// Private root containing materialized custom builder root filesystems.
    pub rootfs_root: PathBuf,
    /// Atomically rewritten digest-to-rootfs manifest for operator inspection.
    pub root_manifest: PathBuf,
    /// Reviewed guest bootstrap injected only by the trusted materializer.
    pub guest_init: PathBuf,
    /// Lease duration for preparation and materialization claims.
    pub lease: Duration,
    /// Poll interval used when no durable OCI job is immediately available.
    pub poll_interval: Duration,
}

/// Complete configuration consumed by the composition root.
#[derive(Clone)]
pub struct AppConfig {
    /// Runtime `PostgreSQL` connection string.
    pub database_url: String,
    /// NATS server connection string.
    pub nats_url: String,
    /// HTTP address for the API and Git transport.
    pub http_listen: SocketAddr,
    /// Domain-separated HS256 key for short-lived mediator assertions.
    pub rpc_mediator_signing_key: [u8; 32],
    /// Canonical root containing bare repositories.
    pub repository_root: PathBuf,
    /// Absolute native `git-http-backend` executable.
    pub git_http_backend: PathBuf,
    /// Absolute host-owned runtime Git `pre-receive` executable.
    pub git_pre_receive_hook: PathBuf,
    /// Git transaction limits.
    pub git_http_limits: GitHttpLimits,
    /// OIDC verifier settings.
    pub oidc: OidcConfig,
    /// Forge-owned OCI registry token-service settings.
    pub registry: RegistryConfig,
    /// Optional repository-gateway edge owned by the same daemon process.
    pub gateway_edge: Option<GatewayEdgeConfig>,
    /// Local persistent-volume settings.
    pub volumes: LocalVolumeConfig,
    /// Exact-commit workspace and durable result storage settings.
    pub workspaces: LocalWorkspaceConfig,
    /// Exact release-artifact and host-context runtime filesystem settings.
    pub run_runtime: LocalRunRuntimeConfig,
    /// Private persistent root for encrypted, temporary authority handoffs.
    pub runtime_authority_handoff_root: PathBuf,
    /// Host-loaded encryption key for runtime-authority handoff envelopes.
    pub runtime_authority_handoff_key: [u8; 32],
    /// Maximum lifetime of one exact runtime-authority session.
    pub runtime_authority_session_ttl: Duration,
    /// Private transient root for isolated build source and output trees.
    pub build_workspace_root: PathBuf,
    /// Maximum wall-clock time for one isolated build.
    pub build_timeout: Duration,
    /// Memory-backed ephemeral secret mount settings.
    pub secret_mounts: EphemeralSecretConfig,
    /// Host-loaded versioned wrapping keys.
    pub secret_keys: LocalKeyProvider,
    /// Private provider-facing semantic broker socket.
    pub secret_broker_socket: PathBuf,
    /// Host-only semantic provider adapter.
    pub secret_broker_adapter: Arc<dyn BrokerAdapter>,
    /// VM implementation selected for this process.
    pub vm_backend: VmBackendConfig,
    /// Immutable image references resolved to provider-neutral roots.
    pub root_images: BTreeMap<String, RootFilesystem>,
    /// Optional local worker for repository-owned OCI builders.
    pub oci_builder: Option<OciBuilderWorkerConfig>,
    /// Current launch-time resource and network ceiling.
    pub runtime_policy: RuntimePolicy,
    /// State-volume capacity provisioned per agent.
    pub agent_state_capacity_bytes: u64,
    /// Maximum concurrently handled NATS commands.
    pub worker_concurrency: usize,
    /// Outbox polling interval.
    pub outbox_poll_interval: Duration,
    /// Records processed by each publisher pass.
    pub outbox_batch_size: i64,
    /// Maximum time allowed for readiness.
    pub startup_timeout: Duration,
    /// Maximum time allowed for graceful task draining.
    pub shutdown_timeout: Duration,
}

impl AppConfig {
    fn validate(&self) -> Result<(), AppError> {
        if !self.git_http_backend.is_absolute() {
            return Err(AppError::Configuration(String::from(
                "git_http_backend must be absolute",
            )));
        }
        if !self.git_pre_receive_hook.is_absolute()
            || self.git_pre_receive_hook.file_name() != Some(std::ffi::OsStr::new("pre-receive"))
        {
            return Err(AppError::Configuration(String::from(
                "git_pre_receive_hook must be an absolute path named pre-receive",
            )));
        }
        if self.rpc_mediator_signing_key == [0; 32] {
            return Err(AppError::Configuration(String::from(
                "RPC mediator authentication key must not be all-zero",
            )));
        }
        if !self.secret_broker_socket.is_absolute() {
            return Err(AppError::Configuration(String::from(
                "secret_broker_socket must be absolute",
            )));
        }
        let backend = std::fs::metadata(&self.git_http_backend).map_err(|error| {
            AppError::Configuration(format!("git_http_backend cannot be inspected: {error}"))
        })?;
        if !backend.is_file() || backend.permissions().mode() & 0o111 == 0 {
            return Err(AppError::Configuration(String::from(
                "git_http_backend must be an executable file",
            )));
        }
        let hook = std::fs::symlink_metadata(&self.git_pre_receive_hook).map_err(|error| {
            AppError::Configuration(format!("git_pre_receive_hook cannot be inspected: {error}"))
        })?;
        if hook.file_type().is_symlink()
            || !hook.is_file()
            || hook.permissions().mode() & 0o111 == 0
        {
            return Err(AppError::Configuration(String::from(
                "git_pre_receive_hook must be a non-symlink executable file",
            )));
        }
        if !self.repository_root.is_absolute() {
            return Err(AppError::Configuration(String::from(
                "repository_root must be absolute",
            )));
        }
        if !self.build_workspace_root.is_absolute() || self.build_timeout.is_zero() {
            return Err(AppError::Configuration(String::from(
                "isolated build root must be absolute and timeout must be positive",
            )));
        }
        if !self.runtime_authority_handoff_root.is_absolute()
            || self.runtime_authority_handoff_key == [0; 32]
            || self.runtime_authority_session_ttl.is_zero()
        {
            return Err(AppError::Configuration(String::from(
                "runtime authority handoff root/key and positive session TTL are required",
            )));
        }
        self.validate_oci_builder()?;
        self.validate_gateway_edge()?;
        self.validate_root_images()?;
        if self.runtime_policy.version.trim().is_empty()
            || self.runtime_policy.max_vcpus == 0
            || self.runtime_policy.max_memory_mib == 0
        {
            return Err(AppError::Configuration(String::from(
                "runtime policy version and positive resource ceilings are required",
            )));
        }
        if self.worker_concurrency == 0 {
            return Err(AppError::Configuration(String::from(
                "worker_concurrency must be greater than zero",
            )));
        }
        if self.outbox_batch_size <= 0 {
            return Err(AppError::Configuration(String::from(
                "outbox_batch_size must be greater than zero",
            )));
        }
        self.validate_registry()?;
        if self.startup_timeout.is_zero() || self.shutdown_timeout.is_zero() {
            return Err(AppError::Configuration(String::from(
                "startup and shutdown timeouts must be greater than zero",
            )));
        }
        Ok(())
    }

    fn validate_registry(&self) -> Result<(), AppError> {
        if self.registry.reconciliation_lease.is_zero()
            || self.registry.reconciliation_interval.is_zero()
        {
            return Err(AppError::Configuration(String::from(
                "registry reconciliation durations must be greater than zero",
            )));
        }
        Ok(())
    }

    fn validate_gateway_edge(&self) -> Result<(), AppError> {
        let Some(gateway) = &self.gateway_edge else {
            return Ok(());
        };
        if !gateway.dispatcher_listen.ip().is_loopback()
            || gateway.public_authority.trim().is_empty()
        {
            return Err(AppError::Configuration(String::from(
                "gateway dispatcher must bind loopback and use a non-empty public authority",
            )));
        }
        LocalCaddyAdministration::new(&gateway.caddy_admin_url)
            .map_err(|error| AppError::Configuration(error.to_string()))?;
        LocalCaddyConfigurationTemplate::new(
            &gateway.caddy_configuration_template,
            gateway.caddy_server_name.clone(),
        )
        .map_err(|error| AppError::Configuration(error.to_string()))?;
        Ok(())
    }

    fn validate_root_images(&self) -> Result<(), AppError> {
        if self.root_images.is_empty() {
            return Err(AppError::Configuration(String::from(
                "at least one root image mapping is required",
            )));
        }
        for (reference, root) in &self.root_images {
            OciImageReference::parse(reference.clone()).map_err(|error| {
                AppError::Configuration(format!(
                    "root image reference {reference:?} is not digest-pinned: {error}"
                ))
            })?;
            let (path, expected_directory) = match root {
                RootFilesystem::Directory { host_path } => (host_path, true),
                RootFilesystem::Disk { host_path, .. } => (host_path, false),
                _ => {
                    return Err(AppError::Configuration(format!(
                        "root image {reference:?} uses an unsupported filesystem variant"
                    )));
                }
            };
            if !path.is_absolute() {
                return Err(AppError::Configuration(format!(
                    "root image {reference:?} materialization path must be absolute"
                )));
            }
            let metadata = std::fs::symlink_metadata(path).map_err(|error| {
                AppError::Configuration(format!(
                    "root image {reference:?} materialization path cannot be inspected: {error}"
                ))
            })?;
            if metadata.file_type().is_symlink() || metadata.is_dir() != expected_directory {
                let expected = if expected_directory {
                    "a non-symlink directory"
                } else {
                    "a non-symlink disk file"
                };
                return Err(AppError::Configuration(format!(
                    "root image {reference:?} materialization path must be {expected}"
                )));
            }
        }
        Ok(())
    }

    fn validate_oci_builder(&self) -> Result<(), AppError> {
        let Some(worker) = &self.oci_builder else {
            return Ok(());
        };
        if worker.runtime.repository_root != self.repository_root
            || !worker.rootfs_root.is_absolute()
            || !worker.root_manifest.is_absolute()
            || !worker.guest_init.is_absolute()
            || !worker.verification_root.is_absolute()
            || !worker.scratch_root.is_absolute()
            || !worker.mkfs_ext4.is_absolute()
            || worker.vm_resources.vcpus == 0
            || worker.vm_resources.memory_mib == 0
            || worker.lease.is_zero()
            || worker.poll_interval.is_zero()
        {
            return Err(AppError::Configuration(String::from(
                "OCI builder roots, manifest, and durations must be explicit and valid",
            )));
        }
        if let VmBackendConfig::Libkrun(provider) = &self.vm_backend
            && !provider.image_roots.contains(&worker.rootfs_root)
        {
            return Err(AppError::Configuration(String::from(
                "libkrun image roots must include the OCI builder rootfs root",
            )));
        }
        for reference in [&worker.builder_vm_image, &worker.verifier_vm_image] {
            if !self.root_images.contains_key(reference.as_str()) {
                return Err(AppError::Configuration(String::from(
                    "OCI operational VM image is not present in the root image manifest",
                )));
            }
        }
        Ok(())
    }
}

/// Constructed application whose external tasks have not started.
pub struct HephaestusApp {
    pool: PgPool,
    application_pool: PgPool,
    service_log_pool: PgPool,
    nats_client: async_nats::Client,
    jetstream: async_nats::jetstream::Context,
    forge: Arc<PgForgeRepository>,
    storage: Arc<GitStorage>,
    identity_store: Arc<PostgresIdentityStore>,
    git_authenticator: Arc<dyn GitAuthenticator>,
    git_authorizer: Arc<PostgresGitAuthorizer>,
    git_backend: PathBuf,
    git_pre_receive_hook: PathBuf,
    git_limits: GitHttpLimits,
    registry: RegistryConfig,
    http_listen: SocketAddr,
    run_repository: Arc<PgRunRepository>,
    mailbox_repository: Arc<PostgresMailboxRepository>,
    review_repository: Arc<PostgresReviewRepository>,
    review_control: ReviewControlService,
    orchestrator: Arc<RunOrchestrator>,
    build_executor: Arc<BuildExecutor>,
    oci_builder_workers: Option<Arc<OciBuilderWorkers>>,
    artifact_store: LocalArtifactStore,
    result_artifact_root: PathBuf,
    release_service: Arc<ReleaseService>,
    update_completion: Arc<UpdateRunCompletion>,
    secret_service: Arc<SecretService<LocalKeyProvider>>,
    rpc_mediator_signing_key: [u8; 32],
    internal_platform_policy: release_domain::RuntimePolicy,
    internal_platform_policy_version: String,
    secret_broker_socket: PathBuf,
    secret_broker_executor: Arc<dyn BrokerExecutor>,
    service_log_maintenance: Arc<GatewayServiceLogMaintenanceScheduler>,
    gateway_edge: Option<GatewayEdgeRuntime>,
    worker_concurrency: usize,
    outbox_poll_interval: Duration,
    outbox_batch_size: i64,
    startup_timeout: Duration,
    shutdown_timeout: Duration,
}

/// Runtime-owned dependencies for the optional shared-Caddy gateway edge.
struct GatewayEdgeRuntime {
    authority: PostgresGatewayEdgeAuthority,
    recovery_authority: PostgresGatewayEdgeAuthority,
    service_supervisor_context: Arc<GatewayServiceSupervisorContext>,
    service_claim_resolution: Arc<dyn GatewayServiceClaimResolutionStore>,
    service_expired_claim_recovery: Arc<dyn GatewayServiceExpiredClaimRecovery>,
    service_boot_context: GatewayServiceBootRecoveryContext,
    service_log_writer: GatewayServiceLogWriterConfig,
    provider: Arc<dyn gateway_edge::GatewayProvider>,
    dispatcher: Arc<dyn GatewayRequestDispatcher>,
    dispatcher_listen: SocketAddr,
    public_authority: String,
}

/// Narrow provider adapter used only after the gateway release resolver has
/// selected an exact immutable launch specification.
#[derive(Clone)]
struct ProviderGatewayRuntimeLauncher {
    provider: Arc<dyn VmProvider>,
}

#[async_trait]
impl GatewayRuntimeLauncher for ProviderGatewayRuntimeLauncher {
    async fn provision_gateway(
        &self,
        spec: VmSpec,
    ) -> Result<Arc<dyn VmInstance>, gateway_edge::GatewayEdgeError> {
        self.provider
            .provision(spec)
            .await
            .map_err(|_| gateway_edge::GatewayEdgeError::HandlerUnavailable)
    }
}

/// Composition adapter that gives gateway `PostgreSQL` authority the same
/// verified local release-artifact lifecycle used by ordinary runs.
#[derive(Clone)]
struct LocalGatewayReleaseMaterializer {
    runtime: LocalGatewayReleaseRuntime,
}

impl GatewayReleaseMaterializer for LocalGatewayReleaseMaterializer {
    fn prepare(
        &self,
        invocation_id: Uuid,
        artifacts: &[GatewayReleaseArtifact],
        parameters: &serde_json::Value,
    ) -> Result<Vec<VmMount>, gateway_edge::GatewayEdgeError> {
        let artifacts = artifacts
            .iter()
            .map(|artifact| RunRuntimeArtifact {
                path: artifact.path.clone(),
                kind: match artifact.kind {
                    GatewayReleaseArtifactKind::Executable => RunRuntimeArtifactKind::Executable,
                    GatewayReleaseArtifactKind::File => RunRuntimeArtifactKind::File,
                    GatewayReleaseArtifactKind::Manifest => RunRuntimeArtifactKind::Manifest,
                },
                mode: artifact.mode,
                content_hash: artifact.content_hash,
                size_bytes: artifact.size_bytes,
                storage_key: artifact.storage_key,
            })
            .collect::<Vec<_>>();
        self.runtime
            .prepare(invocation_id, &artifacts, parameters)
            .map_err(|_| gateway_edge::GatewayEdgeError::HandlerUnavailable)
    }

    fn destroy(&self, invocation_id: Uuid) -> Result<(), gateway_edge::GatewayEdgeError> {
        self.runtime
            .destroy(invocation_id)
            .map_err(|_| gateway_edge::GatewayEdgeError::HandlerUnavailable)
    }
}

impl GatewayServiceMaterializer for LocalGatewayReleaseMaterializer {
    fn prepare_service(
        &self,
        identity: GatewayServiceIdentity,
        artifacts: &[GatewayServiceArtifact],
        parameters: &serde_json::Value,
    ) -> Result<Vec<VmMount>, gateway_edge::GatewayEdgeError> {
        let identity = LocalGatewayServiceIdentity {
            instance_id: identity.instance_id,
            gateway_id: identity.gateway_id,
            revision_id: identity.revision_id,
        };
        let artifacts = artifacts
            .iter()
            .map(|artifact| RunRuntimeArtifact {
                path: artifact.path.clone(),
                kind: match artifact.kind {
                    GatewayServiceArtifactKind::Executable => RunRuntimeArtifactKind::Executable,
                    GatewayServiceArtifactKind::File => RunRuntimeArtifactKind::File,
                    GatewayServiceArtifactKind::Manifest => RunRuntimeArtifactKind::Manifest,
                },
                mode: artifact.mode,
                content_hash: artifact.content_hash,
                size_bytes: artifact.size_bytes,
                storage_key: artifact.storage_key,
            })
            .collect::<Vec<_>>();
        self.runtime
            .prepare_service(identity, &artifacts, parameters)
            .map_err(|_| gateway_edge::GatewayEdgeError::HandlerUnavailable)
    }

    fn destroy_service(
        &self,
        identity: GatewayServiceIdentity,
    ) -> Result<(), gateway_edge::GatewayEdgeError> {
        self.runtime
            .destroy_service(LocalGatewayServiceIdentity {
                instance_id: identity.instance_id,
                gateway_id: identity.gateway_id,
                revision_id: identity.revision_id,
            })
            .map_err(|_| gateway_edge::GatewayEdgeError::HandlerUnavailable)
    }
}

struct OciBuilderWorkers {
    preparation: OciImageProductionWorker<
        PgOciImageProductionJobStore,
        LocalOciRuntime,
        VmPublishedOciEngine<
            PgRepositoryOciImagePublicationStore,
            InternalRegistryTokens,
            SystemCommandRunner,
        >,
    >,
    materialization: RootfsMaterializationWorker<PgOciImageProductionJobStore, LocalOciRuntime>,
    manifest: PathBuf,
    manifest_dirty: AtomicBool,
    rootfs_root: PathBuf,
    image_filesystems: Arc<RwLock<BTreeMap<String, RootFilesystem>>>,
    poll_interval: Duration,
}

#[derive(Clone)]
struct InternalRegistryTokens {
    issuer: Arc<registry_token::RegistryTokenIssuer>,
}

impl InternalRegistryTokens {
    fn issue(
        &self,
        namespace: &RegistryNamespace,
        actions: RepositoryActions,
        action_text: &str,
        subject: &str,
    ) -> Result<IssuedToken, ()> {
        let repository = namespace
            .as_str()
            .parse::<RepositoryName>()
            .map_err(|_| ())?;
        let request = ScopeRequest::parse(
            self.issuer.service().as_str(),
            &format!("repository:{repository}:{action_text}"),
        )
        .map_err(|_| ())?;
        let mut authorization = RegistryAuthorizationDecision::deny_all();
        authorization.grant(repository, actions);
        let now =
            u64::try_from(time::OffsetDateTime::now_utc().unix_timestamp()).map_err(|_| ())?;
        self.issuer
            .issue(
                subject.parse::<TokenSubject>().map_err(|_| ())?,
                &request,
                &authorization,
                UnixTimestamp::new(now),
            )
            .map_err(|_| ())
    }
}

#[async_trait]
impl RegistryPublisherTokenIssuer for InternalRegistryTokens {
    async fn issue_pull_push(
        &self,
        intent: &registry_domain::PublicationIntent,
    ) -> Result<IssuedToken, OciWorkerError> {
        self.issue(
            intent.reference().namespace(),
            RepositoryActions::pull_push(),
            "pull,push",
            "workload:repository-builder",
        )
        .map_err(|()| OciWorkerError::RegistryPublication)
    }
}

#[async_trait]
impl RegistryPullTokenProvider for InternalRegistryTokens {
    async fn issue_pull_token(
        &self,
        namespace: &RegistryNamespace,
    ) -> Result<IssuedToken, ZotClientError> {
        self.issue(
            namespace,
            RepositoryActions::pull(),
            "pull",
            "workload:registry-reconciler",
        )
        .map_err(|()| ZotClientError::Unavailable)
    }
}

#[derive(Clone)]
struct PostgresRegistryReconciliation {
    store: PgRegistryStore,
}

#[async_trait]
impl NotificationInbox for PostgresRegistryReconciliation {
    async fn claim(
        &self,
        lease: Duration,
    ) -> Result<Option<ClaimedNotification>, ReconciliationPortError> {
        self.store
            .claim_notification(lease)
            .await
            .map(|claim| {
                claim.map(|claim| ClaimedNotification {
                    id: claim.id,
                    lease_token: claim.claim_token,
                    repository_path: claim.repository_path,
                    namespace: claim.namespace,
                    target: claim.target.map(|target| ObservedTarget {
                        digest: target.digest,
                        media_type: target.media_type,
                    }),
                })
            })
            .map_err(|_| ReconciliationPortError)
    }

    async fn complete(
        &self,
        claim: &ClaimedNotification,
        completion: NotificationCompletion,
    ) -> Result<(), ReconciliationPortError> {
        let completion = match completion {
            NotificationCompletion::Processed => PgNotificationCompletion::Processed,
            NotificationCompletion::Rejected { failure_code } => {
                PgNotificationCompletion::Rejected { failure_code }
            }
        };
        self.store
            .complete_notification(claim.id, claim.lease_token, completion)
            .await
            .map_err(|_| ReconciliationPortError)
    }
}

#[async_trait]
impl PublicationIntents for PostgresRegistryReconciliation {
    async fn for_namespace(
        &self,
        namespace: &RegistryNamespace,
    ) -> Result<Vec<registry_domain::PublicationIntent>, ReconciliationPortError> {
        self.store
            .list_for_namespace(namespace)
            .await
            .map_err(|_| ReconciliationPortError)
    }

    async fn all(
        &self,
    ) -> Result<Vec<registry_domain::PublicationIntent>, ReconciliationPortError> {
        self.store
            .list_all()
            .await
            .map_err(|_| ReconciliationPortError)
    }
}

#[async_trait]
impl ReconciliationActionExecutor for PostgresRegistryReconciliation {
    async fn apply(&self, action: &ReconciliationAction) -> Result<(), ReconciliationPortError> {
        match action {
            ReconciliationAction::RecordVerified {
                intent_id,
                verification,
            } => {
                self.store
                    .record_verified(*intent_id, verification.clone())
                    .await
                    .map_err(|_| ReconciliationPortError)?;
            }
            ReconciliationAction::MarkMissing { intent_id, reason } => {
                self.store
                    .mark_missing(*intent_id)
                    .await
                    .map_err(|_| ReconciliationPortError)?;
                tracing::warn!(publication_id = %intent_id, ?reason, "registry publication failed closed");
            }
            ReconciliationAction::RestoreVerified {
                intent_id,
                verification,
            } => {
                self.store
                    .restore_verified(*intent_id, verification)
                    .await
                    .map_err(|_| ReconciliationPortError)?;
            }
            ReconciliationAction::ObservedDifferentTarget { namespace } => {
                tracing::warn!(namespace = %namespace, "Zot notification target did not match a publication intent");
            }
            ReconciliationAction::OrphanNamespace { repository_path } => {
                tracing::warn!(
                    repository_path,
                    "Zot notification addressed an unowned namespace"
                );
            }
            ReconciliationAction::Investigate { intent_id, reason } => {
                tracing::warn!(publication_id = %intent_id, ?reason, "registry publication requires investigation");
            }
        }
        Ok(())
    }
}

struct PostgresRegistryScopeAuthorizer {
    store: PgRegistryStore,
}

#[async_trait]
impl RegistryScopeAuthorizer for PostgresRegistryScopeAuthorizer {
    async fn authorize(
        &self,
        identity: &identity_domain::AuthenticatedIdentity,
        request: &registry_token::ScopeRequest,
    ) -> Result<RegistryAuthorizationDecision, RegistryAuthorizationError> {
        let mut decision = RegistryAuthorizationDecision::deny_all();
        for scope in request.scopes() {
            if !scope.actions().contains(RegistryAction::Pull) {
                continue;
            }
            let Ok(namespace) = RegistryNamespace::parse(scope.repository().as_str().to_owned())
            else {
                continue;
            };
            if self
                .store
                .authorize_user_pull(identity, &namespace)
                .await
                .map_err(|_| RegistryAuthorizationError)?
            {
                // Human token exchange is deliberately pull-only. Trusted
                // publishers receive push grants through the worker boundary.
                decision.grant(
                    scope.repository().clone(),
                    registry_token::RepositoryActions::pull(),
                );
            }
        }
        Ok(decision)
    }
}

struct PostgresRegistryNotificationInbox {
    store: PgRegistryStore,
}

#[async_trait]
impl RegistryNotificationInbox for PostgresRegistryNotificationInbox {
    async fn ingest(
        &self,
        observation: NotificationObservation,
    ) -> Result<InboxDisposition, RegistryInboxError> {
        let target =
            observation
                .digest()
                .zip(observation.media_type())
                .map(|(digest, media_type)| RegistryNotificationTarget {
                    digest: digest.clone(),
                    media_type: media_type.clone(),
                });
        let receipt = self
            .store
            .ingest_notification(NewRegistryNotification {
                event_key: observation.idempotency_key().as_str().to_owned(),
                repository_path: observation.repository().as_str().to_owned(),
                action: match observation.action() {
                    NotificationAction::Push => RegistryNotificationAction::Push,
                    NotificationAction::Delete => RegistryNotificationAction::Delete,
                },
                target,
                occurred_at: observation.occurred_at(),
                payload_sha256: *observation.payload_sha256().as_bytes(),
            })
            .await
            .map_err(|_| RegistryInboxError)?;
        Ok(if receipt.duplicate {
            InboxDisposition::Duplicate
        } else {
            InboxDisposition::Accepted
        })
    }
}

async fn registry_caller_authentication(
    axum::extract::State(authenticator): axum::extract::State<Arc<dyn GitAuthenticator>>,
    mut request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let credential = request
        .headers()
        .get(http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let principal = authenticator
        .authenticate(credential.as_deref(), identity_domain::RequestId::new())
        .await;
    let Ok(principal) = principal else {
        let mut response = axum::response::Response::new(axum::body::Body::empty());
        *response.status_mut() = http::StatusCode::UNAUTHORIZED;
        return response;
    };
    let Some(identity) = principal.human_identity().cloned() else {
        let mut response = axum::response::Response::new(axum::body::Body::empty());
        *response.status_mut() = http::StatusCode::UNAUTHORIZED;
        return response;
    };
    request.headers_mut().remove(http::header::AUTHORIZATION);
    request.extensions_mut().insert(identity);
    next.run(request).await
}

impl OciBuilderWorkers {
    fn initialize(
        pool: PgPool,
        config: OciBuilderWorkerConfig,
        token_issuer: Arc<registry_token::RegistryTokenIssuer>,
        provider: Arc<dyn VmProvider>,
        root_images: &BTreeMap<String, RootFilesystem>,
        image_filesystems: Arc<RwLock<BTreeMap<String, RootFilesystem>>>,
    ) -> Result<Self, AppError> {
        if !config.root_manifest.is_absolute() || config.poll_interval.is_zero() {
            return Err(AppError::Configuration(String::from(
                "OCI builder manifest path must be absolute and poll interval must be positive",
            )));
        }
        let runtime = LocalOciRuntime::initialize(config.runtime)
            .map_err(component("OCI local runtime configuration"))?;
        let builder_root = root_images
            .get(config.builder_vm_image.as_str())
            .cloned()
            .ok_or_else(|| {
                AppError::Configuration(String::from("OCI builder VM root is unavailable"))
            })?;
        let verifier_root = root_images
            .get(config.verifier_vm_image.as_str())
            .cloned()
            .ok_or_else(|| {
                AppError::Configuration(String::from("OCI verifier VM root is unavailable"))
            })?;
        let operation = VmOciOperation::initialize(
            provider,
            VmOciOperationConfig {
                builder_root,
                verifier_root,
                candidate_root: runtime.output_root().to_path_buf(),
                scratch_root: config.scratch_root,
                mkfs_ext4: config.mkfs_ext4,
                verification_root: config.verification_root,
                resources: config.vm_resources,
                workload_phase_timing: config.workload_phase_timing,
            },
        )
        .map_err(component("OCI VM operation configuration"))?;
        let publication_store = PgRepositoryOciImagePublicationStore::new(
            pool.clone(),
            PgRegistryStore::new(pool.clone()),
            config.publisher.authority().clone(),
            config.publication_policy_version,
            config.publication_policy,
        );
        let publisher = ForgeZotOciPublisher::new(
            runtime.clone(),
            None,
            publication_store,
            InternalRegistryTokens {
                issuer: token_issuer,
            },
            ControlledOciPublisher::new(config.publisher, SystemCommandRunner),
        );
        let preparation = OciImageProductionWorker::new(
            PgOciImageProductionJobStore::new(pool.clone()),
            runtime.clone(),
            VmPublishedOciEngine::new(operation, publisher),
            config.preparation_worker_name,
            config.materialization_worker_name.clone(),
            config.lease,
        )
        .map_err(component("OCI preparation worker configuration"))?;
        let materialization = RootfsMaterializationWorker::new(
            PgOciImageProductionJobStore::new(pool),
            runtime,
            config.materialization_worker_name,
            config.rootfs_root.clone(),
            config.lease,
        )
        .and_then(|worker| worker.with_guest_init(config.guest_init))
        .map_err(component("OCI materialization worker configuration"))?;
        Ok(Self {
            preparation,
            materialization,
            manifest: config.root_manifest,
            manifest_dirty: AtomicBool::new(false),
            rootfs_root: config.rootfs_root,
            image_filesystems,
            poll_interval: config.poll_interval,
        })
    }

    async fn refresh_image_filesystems(&self) -> Result<(), OciWorkerError> {
        let roots = self.materialization.materialized_roots().await?;
        refresh_image_filesystem_cache(&roots, &self.rootfs_root, &self.image_filesystems)
    }
}

fn refresh_image_filesystem_cache(
    roots: &[MaterializedRoot],
    rootfs_root: &std::path::Path,
    image_filesystems: &Arc<RwLock<BTreeMap<String, RootFilesystem>>>,
) -> Result<(), OciWorkerError> {
    {
        let mut image_filesystems = image_filesystems
            .write()
            .map_err(|_| OciWorkerError::InvalidConfiguration)?;
        for root in roots {
            let canonical =
                std::fs::canonicalize(&root.root_path).map_err(OciWorkerError::Filesystem)?;
            let metadata =
                std::fs::symlink_metadata(&canonical).map_err(OciWorkerError::Filesystem)?;
            if !canonical.starts_with(rootfs_root)
                || metadata.file_type().is_symlink()
                || !metadata.is_dir()
            {
                return Err(OciWorkerError::UnsafeMaterializationPath);
            }
            image_filesystems.insert(
                root.image_reference.to_string(),
                RootFilesystem::Directory {
                    host_path: canonical,
                },
            );
        }
    }
    Ok(())
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
    // Keeping dependency construction in the composition root makes the
    // production security boundaries directly auditable.
    #[allow(clippy::too_many_lines)]
    pub async fn build(mut config: AppConfig) -> Result<Self, AppError> {
        config.validate()?;
        let gateway_service_host_id = config.volumes.host_id.clone();
        if let VmBackendConfig::Libkrun(provider) = &mut config.vm_backend {
            if provider
                .broker_socket_path
                .as_ref()
                .is_some_and(|path| path != &config.secret_broker_socket)
            {
                return Err(AppError::Configuration(String::from(
                    "libkrun broker socket does not match the application broker",
                )));
            }
            provider.broker_socket_path = Some(config.secret_broker_socket.clone());
        }
        let pool = connect_control_plane(&config.database_url, 20)
            .await
            .map_err(component("PostgreSQL connection"))?;
        verify_database_contract(&pool).await?;
        let application_pool = connect_application(&config.database_url, 10)
            .await
            .map_err(component("PostgreSQL application-role connection"))?;

        let storage = Arc::new(
            GitStorage::initialize(&config.repository_root)
                .await
                .map_err(component("Git storage"))?,
        );
        let forge = Arc::new(
            PgForgeRepository::new(pool.clone(), Arc::clone(&storage))
                .with_authorizer(Arc::new(authz_postgres::PostgresMelangeAuthorizer)),
        );
        let run_repository = Arc::new(PgRunRepository::new(pool.clone()));
        let mailbox_repository = Arc::new(PostgresMailboxRepository::new(pool.clone()));
        let review_repository = Arc::new(PostgresReviewRepository::new(pool.clone()));
        let review_locator = Arc::new(GitRepositoryLocator::new(Arc::clone(&storage)));
        let review_control = ReviewControlService::new(
            Arc::clone(&review_repository) as Arc<dyn review_service::ReviewRepository>,
            review_locator,
        );
        let volume_metadata = Arc::new(PostgresVolumeMetadataRepository::new(pool.clone()));
        let volumes = Arc::new(
            LocalVolumeStore::new(volume_metadata, config.volumes)
                .map_err(component("volume configuration"))?,
        );
        volumes
            .initialize()
            .await
            .map_err(component("volume initialization"))?;
        let build_git_binary = config.workspaces.git_binary.clone();
        let result_artifact_root = config.workspaces.artifact_root.clone();
        let workspace_repository = Arc::new(PgWorkspaceMetadataRepository::new(pool.clone()));
        let mut workspaces = LocalWorkspaceManager::new(
            Arc::clone(&workspace_repository)
                as Arc<dyn workspace_domain::WorkspaceMetadataRepository>,
            Arc::clone(&workspace_repository) as Arc<dyn workspace_domain::ResultRepository>,
            config.workspaces,
        )
        .map_err(component("workspace configuration"))?;
        workspaces
            .initialize()
            .map_err(component("workspace initialization"))?;
        let result_artifact_root = std::fs::canonicalize(result_artifact_root)
            .map_err(component("result artifact root"))?;
        let workspaces = Arc::new(workspaces);
        let release_artifact_root = config.run_runtime.release_artifact_root.clone();
        let run_runtime = Arc::new(
            LocalRunRuntimeManager::initialize(run_repository.clone(), config.run_runtime)
                .map_err(component("run runtime initialization"))?,
        );
        let gateway_release_runtime = LocalGatewayReleaseMaterializer {
            runtime: run_runtime.gateway_release_runtime(),
        };
        let gateway_edge_config = config.gateway_edge.take();
        let gateway_secret_keys = config.secret_keys.clone();
        let gateway_handoff_root = config.runtime_authority_handoff_root.clone();
        let gateway_handoff_key = config.runtime_authority_handoff_key;
        let gateway_root_images = config.root_images.clone();
        // Service-log append and retention use one dedicated worker pool. It
        // is created even when the optional gateway edge is disabled so the
        // retention scheduler has stable ownership and shutdown semantics.
        let service_log_pool = connect_oci_worker(&config.database_url, 2)
            .await
            .map_err(component("service-log PostgreSQL connection"))?;
        let service_log_store = Arc::new(PostgresGatewayServiceLogStore::new(
            service_log_pool.clone(),
        ));
        let service_log_projects: Arc<dyn gateway_edge::GatewayServiceLogMaintenanceProjects> =
            service_log_store.clone();
        let service_log_maintenance_port: Arc<dyn gateway_edge::GatewayServiceLogMaintenance> =
            service_log_store.clone();
        let service_log_maintenance = Arc::new(GatewayServiceLogMaintenanceScheduler::new(
            service_log_projects,
            service_log_maintenance_port,
            gateway_edge::GatewayServiceLogMaintenancePolicy::default(),
        ));
        let (secret_mounts, secret_runtime, secret_service) = build_secret_mount_manager(
            pool.clone(),
            &config.database_url,
            config.secret_keys,
            config.secret_mounts,
        )
        .await?;
        let runtime_git_credentials = PgRuntimeGitCredentialRepository::new(pool.clone());
        let runtime_authority = Arc::new(PgRunAuthorityManager::new(
            pool.clone(),
            runtime_git_credentials.clone(),
            config.runtime_authority_handoff_root,
            config.runtime_authority_handoff_key,
            config.runtime_authority_session_ttl,
        )?);
        let secret_broker_executor: Arc<dyn BrokerExecutor> = Arc::new(ServiceBrokerExecutor::new(
            secret_runtime,
            config.secret_broker_adapter,
        ));

        let provider: Arc<dyn VmProvider> = match config.vm_backend {
            VmBackendConfig::Fake => Arc::new(FakeProvider::new()),
            VmBackendConfig::FixtureResult => Arc::new(ResultFixtureProvider),
            VmBackendConfig::Custom(provider) => provider,
            VmBackendConfig::Libkrun(provider) => {
                Arc::new(LibkrunProvider::new(*provider).map_err(component("libkrun provider"))?)
            }
        };
        let image_filesystems = Arc::new(RwLock::new(config.root_images.clone()));
        let oci_builder_workers = match config.oci_builder.take() {
            Some(worker) => {
                let worker_pool = connect_oci_worker(&config.database_url, 4)
                    .await
                    .map_err(component("OCI worker PostgreSQL connection"))?;
                Some(Arc::new(OciBuilderWorkers::initialize(
                    worker_pool,
                    worker,
                    Arc::clone(&config.registry.token_issuer),
                    Arc::clone(&provider),
                    &config.root_images,
                    Arc::clone(&image_filesystems),
                )?))
            }
            None => None,
        };
        let gateway_edge = if let Some(gateway) = gateway_edge_config {
            // Gateway runtime snapshots and sessions are worker-owned
            // immutable authority records. Keep issuance on a dedicated
            // worker-role pool rather than leaking those writes through the
            // user-scoped control-plane pool.
            let gateway_authority_pool = connect_oci_worker(&config.database_url, 4)
                .await
                .map_err(component("gateway runtime authority PostgreSQL connection"))?;
            let service_owner = GatewayServiceOwner::new(gateway_service_host_id, Uuid::new_v4())
                .map_err(component("gateway service owner"))?;
            let service_registry = GatewayServiceRegistry::new(
                GATEWAY_SERVICE_SERVING_CAPACITY + GATEWAY_SERVICE_REPLACEMENT_CAPACITY,
                GATEWAY_SERVICE_REQUEST_CAPACITY,
            )
            .map_err(component("gateway service registry"))?;
            let issuer_handoff =
                EncryptedFileHandoffStore::new(gateway_handoff_root.clone(), gateway_handoff_key)
                    .map_err(component("gateway runtime authority handoff"))?;
            let issuer: Arc<dyn GatewayRuntimeAuthorityIssuer> =
                Arc::new(PgGatewayRuntimeAuthorityIssuer::new(
                    gateway_authority_pool.clone(),
                    issuer_handoff,
                    authz_postgres::AUTHORIZATION_MODEL_VERSION,
                ));
            let authority = PostgresGatewayEdgeAuthority::new(pool.clone(), gateway_limits())
                .with_runtime_authority(issuer, Duration::from_secs(30))
                .map_err(component("gateway runtime authority"))?;
            let recovery_authority =
                PostgresGatewayEdgeAuthority::new(gateway_authority_pool.clone(), gateway_limits());
            let gateway_release_materializer = Arc::new(gateway_release_runtime);
            let gateway_release_materializer_port: Arc<dyn GatewayReleaseMaterializer> =
                gateway_release_materializer.clone();
            let gateway_service_materializer: Arc<dyn GatewayServiceMaterializer> =
                gateway_release_materializer.clone();
            let service_ownership = Arc::new(PostgresGatewayServiceOwnership::new(
                gateway_authority_pool.clone(),
            ));
            let service_claim_resolution: Arc<dyn GatewayServiceClaimResolutionStore> =
                service_ownership.clone();
            let service_expired_claim_recovery: Arc<dyn GatewayServiceExpiredClaimRecovery> =
                service_ownership.clone();
            let service_failure_store = Arc::new(PostgresGatewayServiceFailureStore::new(
                gateway_authority_pool.clone(),
            ));
            let service_log_writer = GatewayServiceLogWriterConfig::new(
                service_log_store.clone() as Arc<dyn GatewayServiceLogStore>,
                ServiceLogWriterPolicy::default(),
            );
            let service_launch_resolver = Arc::new(
                PostgresGatewayServiceLaunchResolver::new(
                    gateway_authority_pool.clone(),
                    gateway_root_images.clone(),
                )
                .with_service_materializer(Arc::clone(&gateway_service_materializer)),
            );
            let service_targets = Arc::new(PostgresGatewayServiceTargets::new(
                gateway_authority_pool.clone(),
            ));
            let resolver_handoff: Arc<dyn RuntimeHandoffStore> = Arc::new(
                EncryptedFileHandoffStore::new(gateway_handoff_root, gateway_handoff_key)
                    .map_err(component("gateway runtime resolver handoff"))?,
            );
            let releases = PostgresGatewayReleaseResolver::new(
                pool.clone(),
                gateway_root_images.clone(),
                resolver_handoff,
            )
            .with_release_materializer(gateway_release_materializer_port);
            let runtime = GatewayRuntimeService::new(
                releases,
                ProviderGatewayRuntimeLauncher {
                    provider: Arc::clone(&provider),
                },
            );
            let stateless_handler = PrivateHttpVmGatewayHandler::new(runtime);
            let service_supervisor_context = Arc::new(GatewayServiceSupervisorContext {
                owner: service_owner.clone(),
                policy: GatewayServiceSupervisorPolicy::default(),
                ownership: service_ownership.clone(),
                failure_store: service_failure_store.clone(),
                resolver: service_launch_resolver.clone(),
                provider: Arc::clone(&provider),
                targets: service_targets.clone(),
                registry: service_registry.clone(),
                service_authority: gateway.public_authority.clone(),
            });
            let service_policy = service_supervisor_context.policy;
            let service_boot_context = GatewayServiceBootRecoveryContext {
                owner: service_supervisor_context.owner.clone(),
                cleanup_policy: GatewayServiceCleanupDriverPolicy {
                    lease: service_policy.lease,
                    database_timeout: service_policy.instance.probe_timeout,
                },
                shutdown_timeout: service_policy.instance.shutdown_timeout,
                ownership: service_ownership.clone(),
                exact_recovery: service_ownership.clone(),
                targets: service_targets.clone(),
                failure_store: service_failure_store.clone(),
                resolver: service_launch_resolver.clone(),
                provider: Arc::clone(&service_supervisor_context.provider),
            };
            GatewayServiceSupervisor::new(clone_service_supervisor_context(
                &service_supervisor_context,
            ))
            .map_err(component("gateway service supervisor"))?;
            let handler = GatewayServiceHandler::new(
                PostgresGatewayExecutionTargetResolver::new(gateway_authority_pool.clone()),
                stateless_handler,
                service_registry,
                service_owner,
            )
            .map_err(component("gateway service handler"))?;
            let ingress_pool = connect_control_plane(&config.database_url, 4)
                .await
                .map_err(component("gateway secret resolver PostgreSQL connection"))?;
            let inbound: Arc<dyn GatewayInboundSecretResolver> =
                Arc::new(GatewayIngressSecretResolver::new(
                    ingress_pool,
                    EncryptedStore::new(gateway_secret_keys),
                ));
            let dispatcher: Arc<dyn GatewayRequestDispatcher> = Arc::new(
                GatewayDispatcher::new(authority.clone(), handler, authority.clone())
                    .with_inbound_secret_resolver(inbound)
                    .with_mailbox_publisher(Arc::new(PostgresGatewayMailboxPublisher::new(
                        pool.clone(),
                    ))),
            );
            let administration = LocalCaddyAdministration::new(&gateway.caddy_admin_url)
                .map_err(component("gateway Caddy administration"))?;
            let template = LocalCaddyConfigurationTemplate::new(
                &gateway.caddy_configuration_template,
                gateway.caddy_server_name,
            )
            .map_err(component("gateway Caddy configuration template"))?;
            let provider: Arc<dyn gateway_edge::GatewayProvider> = Arc::new(
                LocalCaddyGatewayProvider::new(administration, Arc::clone(&dispatcher))
                    .with_dispatcher_upstream(gateway.dispatcher_listen.to_string())
                    .with_configuration_template(template),
            );
            Some(GatewayEdgeRuntime {
                authority,
                recovery_authority,
                service_supervisor_context,
                service_claim_resolution,
                service_expired_claim_recovery,
                service_boot_context,
                service_log_writer,
                provider,
                dispatcher,
                dispatcher_listen: gateway.dispatcher_listen,
                public_authority: gateway.public_authority,
            })
        } else {
            None
        };
        let release_authorizer = Arc::new(authz_postgres::PostgresMelangeAuthorizer);
        let release_service = Arc::new(ReleaseService::new(
            pool.clone(),
            release_authorizer.clone(),
        ));
        let artifact_store = LocalArtifactStore::new(
            std::fs::canonicalize(release_artifact_root)
                .map_err(component("release artifact store"))?,
        )
        .map_err(component("release artifact store"))?;
        let build_executor = Arc::new(
            BuildExecutor::initialize(
                Arc::new(PgBuildRepository::new(pool.clone())),
                Arc::clone(&provider),
                artifact_store.clone(),
                Arc::clone(&release_service),
                BuildExecutorConfig {
                    workspace_root: config.build_workspace_root,
                    repository_root: config.repository_root.clone(),
                    git_binary: build_git_binary,
                    image_filesystems: Arc::clone(&image_filesystems),
                    timeout: config.build_timeout,
                },
            )
            .map_err(component("isolated build executor"))?,
        );
        let internal_platform_policy = release_domain::RuntimePolicy {
            vcpus: config.runtime_policy.max_vcpus,
            memory_mib: config.runtime_policy.max_memory_mib,
            network: if config.runtime_policy.allow_egress {
                release_domain::NetworkAccess::Egress
            } else if config.runtime_policy.allow_broker_only {
                release_domain::NetworkAccess::BrokerOnly
            } else {
                release_domain::NetworkAccess::Disabled
            },
        };
        let internal_platform_policy_version = config.runtime_policy.version.clone();
        let spec_factory = Arc::new(PgAgentVmSpecFactory {
            pool: pool.clone(),
            root_images: config.root_images,
            runtime_policy: config.runtime_policy,
        });
        let launch_authorizer =
            Arc::new(PgRunLaunchAuthorizer::new(pool.clone(), release_authorizer));
        let update_completion = Arc::new(UpdateRunCompletion {
            pool: pool.clone(),
            releases: Arc::clone(&release_service),
            admission_cursor: Mutex::new(None),
        });
        let mailbox_store: Arc<dyn MailboxDispatchStore> = mailbox_repository.clone();
        let completion = Arc::new(CompositeRunCompletionObserver::new(vec![
            Arc::clone(&update_completion) as Arc<dyn RunCompletionObserver>,
            Arc::new(MailboxRunCompletion::new(Arc::clone(&mailbox_store))),
        ]));
        let orchestrator = Arc::new(
            RunOrchestrator::new(
                run_repository.clone(),
                volumes,
                Arc::clone(&provider),
                spec_factory,
                config.agent_state_capacity_bytes,
            )
            .with_workspace_manager(workspaces)
            .with_runtime_manager(run_runtime)
            .with_launch_authorizer(launch_authorizer)
            .with_resource_observer(Arc::new(MailboxRunResources::new(Arc::clone(
                &mailbox_store,
            ))))
            .with_authority_manager(runtime_authority)
            .with_secret_manager(secret_mounts)
            .with_completion_observer(completion),
        );

        let verifier = Arc::new(OidcVerifier::new(
            config.oidc.issuer,
            &config.oidc.audience,
            config.oidc.algorithm,
            config.oidc.decoding_key,
        ));
        let identity_store = Arc::new(PostgresIdentityStore::new(pool.clone()));
        let oidc_git_authenticator = Arc::new(OidcGitAuthenticator::new(
            verifier,
            Arc::clone(&identity_store) as Arc<dyn identity_application::VerifiedIdentityMapper>,
        ));
        let git_authenticator: Arc<dyn GitAuthenticator> = Arc::new(
            CompositeGitAuthenticator::new(
                oidc_git_authenticator,
                Arc::new(pat_postgres::PostgresPersonalAccessTokenService::new(
                    pool.clone(),
                )),
            )
            .with_runtime_git(Arc::new(RuntimeGitHttpAuthenticator::new(Arc::new(
                runtime_git_credentials,
            )))),
        );
        let git_authorizer = Arc::new(PostgresGitAuthorizer::new(Arc::new(
            authz_postgres::PostgresGitAuthorizer::new(pool.clone()),
        )));
        let nats_client = async_nats::connect(&config.nats_url)
            .await
            .map_err(component("NATS connection"))?;
        let jetstream = async_nats::jetstream::new(nats_client.clone());

        Ok(Self {
            pool,
            application_pool,
            nats_client,
            jetstream,
            forge,
            storage,
            identity_store,
            git_authenticator,
            git_authorizer,
            git_backend: config.git_http_backend,
            git_pre_receive_hook: config.git_pre_receive_hook,
            git_limits: config.git_http_limits,
            registry: config.registry,
            http_listen: config.http_listen,
            service_log_pool,
            run_repository,
            mailbox_repository,
            review_repository,
            review_control,
            orchestrator,
            build_executor,
            oci_builder_workers,
            artifact_store,
            result_artifact_root,
            release_service,
            update_completion,
            secret_service,
            rpc_mediator_signing_key: config.rpc_mediator_signing_key,
            internal_platform_policy,
            internal_platform_policy_version,
            secret_broker_socket: config.secret_broker_socket,
            secret_broker_executor,
            service_log_maintenance,
            gateway_edge,
            worker_concurrency: config.worker_concurrency,
            outbox_poll_interval: config.outbox_poll_interval,
            outbox_batch_size: config.outbox_batch_size,
            startup_timeout: config.startup_timeout,
            shutdown_timeout: config.shutdown_timeout,
        })
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
    // Startup deliberately remains ordered in one method so the readiness
    // barrier and failure cleanup sequence are directly auditable.
    #[allow(clippy::too_many_lines)]
    pub async fn start(self) -> Result<RunningHephaestus, AppError> {
        self.build_executor
            .recover_after_restart()
            .await
            .map_err(component("build recovery"))?;
        self.orchestrator
            .recover_after_restart()
            .await
            .map_err(component("run recovery"))?;
        ensure_forge_jetstream_topology(&self.jetstream)
            .await
            .map_err(component("forge JetStream topology"))?;
        let build_consumer = ensure_build_consumer(&self.jetstream)
            .await
            .map_err(component("build JetStream consumer"))?;
        ensure_release_jetstream_topology(&self.jetstream)
            .await
            .map_err(component("release JetStream topology"))?;
        event_adapter::ensure_topology(&self.jetstream)
            .await
            .map_err(component("product-event JetStream topology"))?;
        let consumer = ensure_jetstream_topology(&self.jetstream)
            .await
            .map_err(component("run JetStream topology"))?;
        let mailbox_consumer = ensure_mailbox_jetstream_topology(&self.jetstream)
            .await
            .map_err(component("mailbox JetStream topology"))?;
        let broker = BrokerServer::bind(
            self.secret_broker_socket,
            Arc::clone(&self.secret_broker_executor),
        )
        .map_err(component("secret broker listener"))?;
        let git = GitHttpService::new(
            Arc::clone(&self.forge),
            Arc::clone(&self.storage),
            self.git_authenticator.clone(),
            self.git_authorizer,
            self.git_backend,
            self.git_limits,
        )
        .and_then(|service| service.with_runtime_receive_hook(self.git_pre_receive_hook))
        .map_err(component("Git HTTP configuration"))?;
        let command_state = application::commands::InternalCommandState::new(
            Arc::clone(&self.release_service),
            Arc::clone(&self.secret_service),
            self.internal_platform_policy.clone(),
            self.internal_platform_policy_version.clone(),
        );
        let rpc = rpc::service(
            rpc::ApplicationDependencies::new(
                self.pool.clone(),
                self.application_pool.clone(),
                Arc::clone(&self.forge),
                Arc::new(event_postgres::PostgresMutationReceiptReader::new(
                    self.pool.clone(),
                )),
                Arc::clone(&self.identity_store) as Arc<dyn IdempotentIdentityResolver>,
            ),
            Arc::clone(&self.storage),
            self.artifact_store.clone(),
            self.result_artifact_root,
            &self.rpc_mediator_signing_key,
            command_state,
            Arc::new(event_adapter::NatsEventWakeups::new(
                self.nats_client.clone(),
            )),
        )
        .map_err(component("Connect RPC configuration"))?;
        let registry_store = PgRegistryStore::new(self.pool.clone());
        let registry_reconciliation_adapter = PostgresRegistryReconciliation {
            store: registry_store.clone(),
        };
        let registry_reconciler = RegistryReconciler::new(
            registry_reconciliation_adapter.clone(),
            registry_reconciliation_adapter.clone(),
            ZotHttpRegistry::new(
                self.registry.zot.clone(),
                Arc::new(InternalRegistryTokens {
                    issuer: Arc::clone(&self.registry.token_issuer),
                }),
            )
            .map_err(component("Zot reconciliation client"))?,
        );
        let registry_reconciliation_lease = self.registry.reconciliation_lease;
        let registry_reconciliation_interval = self.registry.reconciliation_interval;
        let registry_tokens = RegistryTokenHttpService::new(
            Arc::clone(&self.registry.token_issuer),
            Arc::new(PostgresRegistryScopeAuthorizer {
                store: registry_store.clone(),
            }),
        )
        .router()
        .layer(axum::middleware::from_fn_with_state(
            Arc::clone(&self.git_authenticator),
            registry_caller_authentication,
        ));
        let registry_notifications = RegistryNotificationHttpService::new(
            self.registry.notification_callback.clone(),
            Arc::new(PostgresRegistryNotificationInbox {
                store: registry_store.clone(),
            }),
        )
        .router();
        let gateway_listener = if let Some(gateway) = &self.gateway_edge {
            let listener = tokio::net::TcpListener::bind(gateway.dispatcher_listen)
                .await
                .map_err(component("gateway private dispatcher listener"))?;
            let desired = gateway
                .authority
                .desired_configuration()
                .await
                .map_err(component("gateway desired configuration"))?;
            gateway
                .provider
                .reconcile(&desired)
                .await
                .map_err(component("gateway Caddy reconciliation"))?;
            Some((
                listener,
                PrivateGatewayDispatcherState {
                    dispatcher: Arc::clone(&gateway.dispatcher),
                    public_authority: gateway.public_authority.clone(),
                },
            ))
        } else {
            None
        };
        let router = Router::new()
            .route("/healthz", get(|| async { "ok" }))
            .merge(git.router())
            .merge(registry_tokens)
            .merge(registry_notifications)
            .fallback_service(rpc)
            .layer(axum::middleware::from_fn_with_state(
                rpc::MediatorAuthenticator::new(&self.rpc_mediator_signing_key),
                rpc::mediator_identity_middleware,
            ));
        let listener = tokio::net::TcpListener::bind(self.http_listen)
            .await
            .map_err(component("HTTP listener"))?;
        let http_addr = listener
            .local_addr()
            .map_err(component("HTTP listener address"))?;

        if let Some(workers) = &self.oci_builder_workers {
            workers
                .materialization
                .write_manifest(&workers.manifest)
                .await
                .map_err(component("OCI builder root manifest"))?;
            workers
                .refresh_image_filesystems()
                .await
                .map_err(component("OCI builder image cache"))?;
        }

        let cancellation = CancellationToken::new();
        let mut tasks = Vec::with_capacity(10);
        let service_log_maintenance = Arc::clone(&self.service_log_maintenance);
        let service_log_cancel = cancellation.clone();
        tasks.push(tokio::spawn(async move {
            let result = service_log_maintenance
                .run(service_log_cancel.clone())
                .await
                .map_err(|error| error.to_string());
            if result.is_err() {
                service_log_cancel.cancel();
            }
            result
        }));
        let (update_reconcile_ready_tx, update_reconcile_ready_rx) = oneshot::channel();
        let update_reconcile_cancel = cancellation.clone();
        let update_reconcile_observer = Arc::clone(&self.update_completion);
        let update_reconcile_interval = self.outbox_poll_interval;
        tasks.push(tokio::spawn(async move {
            update_admission_reconciliation_loop(
                update_reconcile_observer,
                update_reconcile_cancel,
                update_reconcile_interval,
                update_reconcile_ready_tx,
            )
            .await;
            Ok(())
        }));
        if let Some(gateway) = self.gateway_edge {
            let gateway_reconcile_cancel = cancellation.clone();
            let gateway_authority = gateway.authority.clone();
            let gateway_recovery_authority = gateway.recovery_authority.clone();
            let service_supervisor_context =
                clone_service_supervisor_context(&gateway.service_supervisor_context);
            let service_boot_recovery =
                GatewayServiceBootRecovery::new(gateway.service_boot_context)
                    .map_err(component("gateway service boot recovery"))?;
            let service_claim_resolution = Arc::clone(&gateway.service_claim_resolution);
            let service_expired_claim_recovery =
                Arc::clone(&gateway.service_expired_claim_recovery);
            let service_log_writer = gateway.service_log_writer;
            let service_targets = Arc::clone(&gateway.service_supervisor_context.targets);
            let gateway_provider = Arc::clone(&gateway.provider);
            tasks.push(tokio::spawn(async move {
                gateway_reconciliation_loop_with_context(
                    gateway_authority,
                    gateway_recovery_authority,
                    service_supervisor_context,
                    service_boot_recovery,
                    Some(service_claim_resolution),
                    Some(service_expired_claim_recovery),
                    Some(service_log_writer),
                    service_targets,
                    gateway_provider,
                    gateway_reconcile_cancel,
                )
                .await;
                Ok(())
            }));
        }
        let (broker_ready_tx, broker_ready_rx) = oneshot::channel();
        let broker_cancel = cancellation.clone();
        tasks.push(tokio::spawn(async move {
            if broker_ready_tx.send(()).is_err() {
                return Ok(());
            }
            let result = broker
                .serve(broker_cancel.clone())
                .await
                .map_err(|error| error.to_string());
            if result.is_err() {
                broker_cancel.cancel();
            }
            result
        }));
        let gateway_ready_rx = if let Some((listener, state)) = gateway_listener {
            let (gateway_ready_tx, gateway_ready_rx) = oneshot::channel();
            let gateway_cancel = cancellation.clone();
            tasks.push(tokio::spawn(async move {
                if gateway_ready_tx.send(()).is_err() {
                    return Ok(());
                }
                let result = axum::serve(
                    listener,
                    Router::new()
                        .fallback(any(private_gateway_dispatch))
                        .with_state(state)
                        .into_make_service_with_connect_info::<SocketAddr>(),
                )
                .with_graceful_shutdown(gateway_cancel.clone().cancelled_owned())
                .await
                .map_err(|error| error.to_string());
                if !gateway_cancel.is_cancelled() {
                    gateway_cancel.cancel();
                }
                result
            }));
            Some(gateway_ready_rx)
        } else {
            None
        };
        let (http_ready_tx, http_ready_rx) = oneshot::channel();
        let http_cancel = cancellation.clone();
        tasks.push(tokio::spawn(async move {
            if http_ready_tx.send(()).is_err() {
                return Ok(());
            }
            let graceful = http_cancel.clone();
            let result = axum::serve(listener, router)
                .with_graceful_shutdown(graceful.cancelled_owned())
                .await
                .map_err(|error| error.to_string());
            if !http_cancel.is_cancelled() {
                http_cancel.cancel();
            }
            result
        }));

        let (publisher_ready_tx, publisher_ready_rx) = oneshot::channel();
        let publisher_cancel = cancellation.clone();
        let outbox = OutboxWorker {
            forge_publisher: ForgeNatsOutboxPublisher::new(self.jetstream.clone()),
            release_publisher: ReleaseOutboxPublisher::new(
                self.jetstream.clone(),
                self.pool.clone(),
            ),
            review_publisher: ReviewOutboxPublisher::new(
                self.jetstream.clone(),
                Arc::clone(&self.review_repository) as Arc<dyn review_service::ReviewOutboxStore>,
            ),
            event_publisher: event_adapter::EventPublisher::new(
                self.jetstream.clone(),
                Arc::new(event_postgres::PostgresProductEventOutbox::new(
                    self.pool.clone(),
                )),
                self.rpc_mediator_signing_key,
            ),
            mailbox_publisher: MailboxOutboxPublisher::new(
                self.jetstream.clone(),
                self.mailbox_repository.clone(),
            ),
            forge: Arc::clone(&self.forge),
            poll_interval: self.outbox_poll_interval,
            batch_size: self.outbox_batch_size,
        };
        tasks.push(tokio::spawn(async move {
            outbox.run(publisher_cancel, publisher_ready_tx).await;
            Ok(())
        }));

        let (mailbox_recovery_ready_tx, mailbox_recovery_ready_rx) = oneshot::channel();
        let mailbox_recovery_cancel = cancellation.clone();
        let mailbox_recovery_store: Arc<dyn MailboxDispatchStore> = self.mailbox_repository.clone();
        let mailbox_recovery_interval = self.outbox_poll_interval;
        tasks.push(tokio::spawn(async move {
            let result = mailbox_recovery_loop(
                mailbox_recovery_store,
                mailbox_recovery_interval,
                mailbox_recovery_cancel.clone(),
                mailbox_recovery_ready_tx,
            )
            .await;
            if result.is_err() {
                mailbox_recovery_cancel.cancel();
            }
            result
        }));

        let (secret_reconcile_ready_tx, secret_reconcile_ready_rx) = oneshot::channel();
        let secret_reconcile_cancel = cancellation.clone();
        let secret_reconcile_pool = self.pool.clone();
        let secret_reconcile_orchestrator = Arc::clone(&self.orchestrator);
        let secret_reconcile_interval = self.outbox_poll_interval;
        tasks.push(tokio::spawn(async move {
            let result = secret_revocation_loop(
                secret_reconcile_pool,
                secret_reconcile_orchestrator,
                secret_reconcile_interval,
                secret_reconcile_cancel.clone(),
                secret_reconcile_ready_tx,
            )
            .await;
            if result.is_err() {
                secret_reconcile_cancel.cancel();
            }
            result
        }));

        let (mailbox_consumer_ready_tx, mailbox_consumer_ready_rx) = oneshot::channel();
        let mailbox_consumer_cancel = cancellation.clone();
        let mailbox_handler = NatsMailboxCommandHandler::new(MailboxCommandHandler::new(
            self.mailbox_repository.clone(),
            Arc::clone(&self.orchestrator),
        ));
        let mailbox_concurrency = self.worker_concurrency;
        tasks.push(tokio::spawn(async move {
            let result = mailbox_command_loop(
                mailbox_consumer,
                mailbox_handler,
                mailbox_concurrency,
                mailbox_consumer_cancel.clone(),
                mailbox_consumer_ready_tx,
            )
            .await;
            if result.is_err() {
                mailbox_consumer_cancel.cancel();
            }
            result
        }));

        let (build_ready_tx, build_ready_rx) = oneshot::channel();
        let build_cancel = cancellation.clone();
        let build_executor = Arc::clone(&self.build_executor);
        let build_concurrency = self.worker_concurrency;
        tasks.push(tokio::spawn(async move {
            let result = build_loop(
                build_consumer,
                build_executor,
                build_concurrency,
                build_cancel.clone(),
                build_ready_tx,
            )
            .await;
            if result.is_err() {
                build_cancel.cancel();
            }
            result
        }));

        if let Some(workers) = &self.oci_builder_workers {
            let oci_workers = Arc::clone(workers);
            let oci_cancel = cancellation.clone();
            tasks.push(tokio::spawn(async move {
                oci_builder_loop(oci_workers, oci_cancel).await;
                Ok(())
            }));
        }

        let registry_reconcile_cancel = cancellation.clone();
        tasks.push(tokio::spawn(async move {
            registry_reconciliation_loop(
                registry_reconciler,
                registry_reconciliation_adapter,
                registry_reconciliation_lease,
                registry_reconciliation_interval,
                registry_reconcile_cancel,
            )
            .await;
            Ok(())
        }));

        let (consumer_ready_tx, consumer_ready_rx) = oneshot::channel();
        let consumer_cancel = cancellation.clone();
        let handler = NatsCommandHandler::new(Arc::clone(&self.orchestrator));
        let control_handler = NatsControlHandler::new(self.review_control);
        let concurrency = self.worker_concurrency;
        tasks.push(tokio::spawn(async move {
            let result = command_loop(
                consumer,
                handler,
                control_handler,
                concurrency,
                consumer_cancel.clone(),
                consumer_ready_tx,
            )
            .await;
            if result.is_err() {
                consumer_cancel.cancel();
            }
            result
        }));

        let readiness = async {
            broker_ready_rx
                .await
                .map_err(|_| AppError::Readiness(String::from("secret broker task exited")))?;
            update_reconcile_ready_rx.await.map_err(|_| {
                AppError::Readiness(String::from("update reconciliation task exited"))
            })?;
            http_ready_rx
                .await
                .map_err(|_| AppError::Readiness(String::from("HTTP task exited")))?;
            if let Some(gateway_ready_rx) = gateway_ready_rx {
                gateway_ready_rx.await.map_err(|_| {
                    AppError::Readiness(String::from("gateway private dispatcher task exited"))
                })?;
            }
            publisher_ready_rx
                .await
                .map_err(|_| AppError::Readiness(String::from("outbox task exited")))?;
            mailbox_recovery_ready_rx
                .await
                .map_err(|_| AppError::Readiness(String::from("mailbox recovery task exited")))?;
            secret_reconcile_ready_rx.await.map_err(|_| {
                AppError::Readiness(String::from("secret reconciliation task exited"))
            })?;
            build_ready_rx
                .await
                .map_err(|_| AppError::Readiness(String::from("build task exited")))?;
            consumer_ready_rx
                .await
                .map_err(|_| AppError::Readiness(String::from("consumer task exited")))?;
            mailbox_consumer_ready_rx
                .await
                .map_err(|_| AppError::Readiness(String::from("mailbox consumer task exited")))?;
            Ok::<(), AppError>(())
        };
        match tokio::time::timeout(self.startup_timeout, readiness).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                cancellation.cancel();
                reap_failed_start(tasks).await;
                return Err(error);
            }
            Err(error) => {
                cancellation.cancel();
                reap_failed_start(tasks).await;
                return Err(AppError::Readiness(format!(
                    "startup readiness timed out: {error}"
                )));
            }
        }
        if cancellation.is_cancelled() {
            reap_failed_start(tasks).await;
            return Err(AppError::Readiness(String::from(
                "a supervised task exited during startup",
            )));
        }

        Ok(RunningHephaestus {
            http_addr,
            cancellation,
            tasks,
            pool: self.pool,
            application_pool: self.application_pool,
            service_log_pool: self.service_log_pool,
            nats_client: self.nats_client,
            jetstream: self.jetstream,
            forge: self.forge,
            run_repository: self.run_repository,
            mailbox_repository: self.mailbox_repository,
            review_repository: self.review_repository,
            orchestrator: self.orchestrator,
            outbox_batch_size: self.outbox_batch_size,
            product_event_cursor_key: self.rpc_mediator_signing_key,
            shutdown_timeout: self.shutdown_timeout,
        })
    }
}

const MAX_PRIVATE_GATEWAY_REQUEST_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone)]
struct PrivateGatewayDispatcherState {
    dispatcher: Arc<dyn GatewayRequestDispatcher>,
    public_authority: String,
}

const fn gateway_limits() -> GatewayLimits {
    GatewayLimits {
        max_request_body_bytes: MAX_PRIVATE_GATEWAY_REQUEST_BYTES,
        max_response_body_bytes: MAX_PRIVATE_GATEWAY_REQUEST_BYTES,
        max_request_headers: 128,
        max_response_headers: 128,
        max_path_and_query_bytes: 8 * 1024,
        execution_timeout: Duration::from_secs(30),
    }
}

async fn private_gateway_dispatch(
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    State(state): State<PrivateGatewayDispatcherState>,
    request: Request<Body>,
) -> Response {
    if !peer.ip().is_loopback() {
        return gateway_http_response(
            http::StatusCode::FORBIDDEN,
            http::HeaderMap::new(),
            Bytes::new(),
        );
    }
    let path_and_query = request
        .uri()
        .path_and_query()
        .map_or_else(|| String::from("/"), ToString::to_string);
    let method = request.method().clone();
    let mut headers = request.headers().clone();
    for name in UNTRUSTED_FORWARDING_HEADERS {
        headers.remove(name);
    }
    for name in [
        http::header::CONNECTION,
        http::header::PROXY_AUTHENTICATE,
        http::header::PROXY_AUTHORIZATION,
        http::header::TE,
        http::header::TRAILER,
        http::header::TRANSFER_ENCODING,
        http::header::UPGRADE,
    ] {
        headers.remove(name);
    }
    headers.remove("keep-alive");
    let Ok(body) =
        axum::body::to_bytes(request.into_body(), MAX_PRIVATE_GATEWAY_REQUEST_BYTES).await
    else {
        return gateway_http_response(
            http::StatusCode::PAYLOAD_TOO_LARGE,
            http::HeaderMap::new(),
            Bytes::new(),
        );
    };
    let response = state
        .dispatcher
        .dispatch(GatewayRequest {
            method,
            path_and_query,
            headers,
            body,
            trusted: TrustedRequestMetadata {
                scheme: GatewayScheme::Https,
                authority: state.public_authority,
                client_address: peer.ip(),
                request_id: Uuid::new_v4(),
            },
        })
        .await
        .response;
    gateway_http_response(response.status, response.headers, response.body)
}

fn gateway_http_response(
    status: http::StatusCode,
    headers: http::HeaderMap,
    body: Bytes,
) -> Response {
    let mut response = Response::new(Body::from(body));
    *response.status_mut() = status;
    *response.headers_mut() = headers;
    response
}

const GATEWAY_RECONCILIATION_INTERVAL: Duration = Duration::from_secs(1);
const GATEWAY_CADDY_RECOVERY_INTERVAL: Duration = Duration::from_secs(30);

type GatewayServiceTargetScan = Pin<
    Box<
        dyn Future<Output = Result<GatewayServiceTargetPageResult, gateway_edge::GatewayEdgeError>>
            + Send,
    >,
>;

const SERVICE_TARGET_REFRESH_TIMEOUT: Duration = Duration::from_secs(2);
const SERVICE_CLEANUP_RETRY_INITIAL_BACKOFF: Duration = Duration::from_secs(1);
const SERVICE_CLEANUP_RETRY_MAX_BACKOFF: Duration = Duration::from_secs(30);

struct TrackedServiceJob {
    gateway_id: Uuid,
    revision_id: Uuid,
    handle: gateway_edge::GatewayServiceStartupHandle,
    retirement_requested: bool,
    cleanup_retry_due: Option<Instant>,
    cleanup_retry_backoff: Duration,
    cleanup_retry_attempted: bool,
}

type GatewayServiceTargetRefresh = Pin<
    Box<
        dyn Future<
                Output = (
                    Uuid,
                    Uuid,
                    Uuid,
                    Result<Option<GatewayServiceOwnedTarget>, gateway_edge::GatewayEdgeError>,
                ),
            > + Send,
    >,
>;

fn clone_service_supervisor_context(
    context: &Arc<GatewayServiceSupervisorContext>,
) -> GatewayServiceSupervisorContext {
    GatewayServiceSupervisorContext {
        owner: context.owner.clone(),
        policy: context.policy,
        ownership: Arc::clone(&context.ownership),
        failure_store: Arc::clone(&context.failure_store),
        resolver: Arc::clone(&context.resolver),
        provider: Arc::clone(&context.provider),
        targets: Arc::clone(&context.targets),
        registry: context.registry.clone(),
        service_authority: context.service_authority.clone(),
    }
}

/// Reconstructs Caddy exclusively from authoritative route records. Ordinary
/// passes apply revision cutovers promptly; a bounded forced pass repairs a
/// Caddy process which restarted after this daemon observed the same revision.
// Keep separately owned adapters explicit at this daemon composition boundary.
// The supervisor owns cancellation and cleanup handles until the parent loop
// finishes; keeping it in this named binding makes that lifetime explicit.
#[allow(clippy::significant_drop_tightening, clippy::too_many_arguments)]
async fn gateway_reconciliation_loop_with_context(
    authority: PostgresGatewayEdgeAuthority,
    recovery_authority: PostgresGatewayEdgeAuthority,
    supervisor_context: GatewayServiceSupervisorContext,
    boot_recovery: GatewayServiceBootRecovery,
    service_claim_resolution: Option<Arc<dyn GatewayServiceClaimResolutionStore>>,
    service_expired_claim_recovery: Option<Arc<dyn GatewayServiceExpiredClaimRecovery>>,
    service_log_writer: Option<GatewayServiceLogWriterConfig>,
    service_targets: Arc<dyn gateway_edge::GatewayServiceTargetStore>,
    provider: Arc<dyn GatewayProvider>,
    cancellation: CancellationToken,
) {
    let mut service_supervisor =
        GatewayServiceSupervisor::new(supervisor_context).expect("validated service supervisor");
    if let Some(writer) = service_log_writer {
        service_supervisor = service_supervisor.with_log_writer(writer);
    }
    gateway_reconciliation_loop_with_boot(
        authority,
        recovery_authority,
        service_supervisor,
        Some(boot_recovery),
        service_claim_resolution,
        service_expired_claim_recovery,
        service_targets,
        provider,
        cancellation,
    )
    .await;
}

#[cfg(test)]
async fn gateway_reconciliation_loop(
    authority: PostgresGatewayEdgeAuthority,
    recovery_authority: PostgresGatewayEdgeAuthority,
    supervisor_context: GatewayServiceSupervisorContext,
    provider: Arc<dyn GatewayProvider>,
    cancellation: CancellationToken,
) {
    gateway_reconciliation_loop_with_supervisor(
        authority,
        recovery_authority,
        supervisor_context,
        provider,
        cancellation,
    )
    .await;
}

// Kept as a narrow test-facing composition helper for existing daemon-loop
// tests. Production supplies the already-constructed boot gate below.
#[cfg(test)]
async fn gateway_reconciliation_loop_with_supervisor(
    authority: PostgresGatewayEdgeAuthority,
    recovery_authority: PostgresGatewayEdgeAuthority,
    supervisor_context: GatewayServiceSupervisorContext,
    provider: Arc<dyn GatewayProvider>,
    cancellation: CancellationToken,
) {
    let targets = Arc::clone(&supervisor_context.targets);
    gateway_reconciliation_loop_with_boot(
        authority,
        recovery_authority,
        GatewayServiceSupervisor::new(supervisor_context).expect("validated service supervisor"),
        None,
        None,
        None,
        targets,
        provider,
        cancellation,
    )
    .await;
}

// The single select set is deliberate: it keeps Caddy, recovery, service
// jobs, and shutdown under one parent-owned polling boundary.
// The loop receives separately owned adapters so each parent-polled subsystem
// keeps its cancellation and lifetime boundary explicit.
#[allow(
    clippy::cognitive_complexity,
    clippy::too_many_arguments,
    clippy::too_many_lines
)]
async fn gateway_reconciliation_loop_with_boot(
    authority: PostgresGatewayEdgeAuthority,
    recovery_authority: PostgresGatewayEdgeAuthority,
    mut service_supervisor: GatewayServiceSupervisor,
    mut boot_recovery: Option<GatewayServiceBootRecovery>,
    service_claim_resolution: Option<Arc<dyn GatewayServiceClaimResolutionStore>>,
    service_expired_claim_recovery: Option<Arc<dyn GatewayServiceExpiredClaimRecovery>>,
    service_targets: Arc<dyn gateway_edge::GatewayServiceTargetStore>,
    provider: Arc<dyn GatewayProvider>,
    cancellation: CancellationToken,
) {
    let mut reconcile = tokio::time::interval(GATEWAY_RECONCILIATION_INTERVAL);
    reconcile.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut service_recovery = tokio::time::interval(GATEWAY_RECONCILIATION_INTERVAL);
    service_recovery.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // Avoid an immediate duplicate of the startup reconciliation while still
    // making a daemon-owned Caddy restart recover without operator action.
    let mut recovery = tokio::time::interval_at(
        tokio::time::Instant::now() + GATEWAY_CADDY_RECOVERY_INTERVAL,
        GATEWAY_CADDY_RECOVERY_INTERVAL,
    );
    recovery.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut service_recovery_task: Option<JoinHandle<()>> = None;
    let mut caddy_task = None;
    let mut caddy_recovery_pending = false;
    let mut target_scan: Option<GatewayServiceTargetScan> = None;
    let mut target_scan_after = None;
    let mut target_scan_interval = tokio::time::interval(GATEWAY_RECONCILIATION_INTERVAL);
    target_scan_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut tracked_jobs = HashMap::<Uuid, TrackedServiceJob>::new();
    let mut target_refresh: Option<GatewayServiceTargetRefresh> = None;
    let mut target_refresh_cursor = None;
    let mut target_refresh_interval = tokio::time::interval(GATEWAY_RECONCILIATION_INTERVAL);
    target_refresh_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut cleanup_retry_interval = tokio::time::interval(GATEWAY_RECONCILIATION_INTERVAL);
    cleanup_retry_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut cleanup_retry_cursor = None;
    loop {
        tokio::select! {
            () = cancellation.cancelled() => {
                if let Some(task) = service_recovery_task.take() {
                    task.abort();
                    let _ = task.await;
                }
                if let Some(boot_recovery) = boot_recovery.take() {
                    let boot_shutdown = boot_recovery.shutdown().await;
                    if !boot_shutdown.unresolved.is_empty()
                        || !boot_shutdown.unresolved_claims.is_empty()
                    {
                        tracing::warn!(
                            unresolved = boot_shutdown.unresolved.len()
                                + boot_shutdown.unresolved_claims.len(),
                            "gateway service boot recovery retained unresolved shutdown work"
                        );
                    }
                }
                let shutdown = service_supervisor.shutdown().await;
                if !shutdown.unresolved.is_empty() {
                    tracing::warn!(
                        unresolved = shutdown.unresolved.len(),
                        "gateway service supervisor retained unresolved shutdown work"
                    );
                }
                return;
            },
            _ = reconcile.tick() => {
                if caddy_task.is_none() {
                    caddy_task = Some(Box::pin(reconcile_gateway_once(
                        authority.clone(),
                        Arc::clone(&provider),
                        false,
                    )));
                }
            }
            _ = recovery.tick() => {
                if caddy_task.is_some() {
                    caddy_recovery_pending = true;
                } else {
                    caddy_task = Some(Box::pin(reconcile_gateway_once(
                        authority.clone(),
                        Arc::clone(&provider),
                        true,
                    )));
                }
            }
            _ = async {
                match caddy_task.as_mut() {
                    Some(task) => {
                        task.await;
                        Some(())
                    },
                    None => std::future::pending().await,
                }
            }, if caddy_task.is_some() => {
                caddy_task = None;
                if caddy_recovery_pending {
                    caddy_recovery_pending = false;
                    caddy_task = Some(Box::pin(reconcile_gateway_once(
                        authority.clone(),
                        Arc::clone(&provider),
                        true,
                    )));
                }
            }
            _ = service_recovery.tick(), if service_recovery_task.is_none() => {
                let authority = recovery_authority.clone();
                service_recovery_task = Some(tokio::spawn(async move {
                    match authority
                        .recover_abandoned_service_invocations(OffsetDateTime::now_utc())
                        .await
                    {
                        Ok(recovered) if recovered > 0 => {
                            tracing::info!(recovered, "recovered abandoned gateway service invocations");
                        }
                        Ok(_) => {}
                        Err(error) => {
                            tracing::warn!(%error, "gateway service invocation recovery failed");
                        }
                    }
                }));
            }
            result = async {
                match service_recovery_task.as_mut() {
                    Some(task) => Some(task.await),
                    None => std::future::pending().await,
                }
            }, if service_recovery_task.is_some() => {
                if let Some(Err(error)) = result {
                    tracing::warn!(%error, "gateway service invocation recovery task failed");
                }
                service_recovery_task = None;
            }
            boot_event = async {
                match boot_recovery.as_mut() {
                    Some(recovery) => Some(recovery.poll().await),
                    None => std::future::pending().await,
                }
            }, if boot_recovery.as_ref().is_some_and(|recovery| !recovery.is_complete()) => {
                match boot_event {
                    Some(Ok(gateway_edge::GatewayServiceBootRecoveryEvent::Complete)) => {
                        tracing::info!("gateway service boot recovery gate completed");
                    }
                    Some(Ok(
                        gateway_edge::GatewayServiceBootRecoveryEvent::Pending
                        | gateway_edge::GatewayServiceBootRecoveryEvent::Waiting,
                    )) => {}
                    Some(Err(error)) => {
                        tracing::warn!(%error, "gateway service boot recovery is unavailable");
                    }
                    None => unreachable!("boot poll branch is enabled only with a boot gate"),
                }
            }
            _ = target_scan_interval.tick(),
                if boot_recovery.as_ref().is_some_and(GatewayServiceBootRecovery::is_complete)
                    && target_scan.is_none() =>
            {
                let page = match GatewayServiceTargetPage::new(
                    target_scan_after,
                    gateway_edge::MAX_SERVICE_TARGET_PAGE_SIZE,
                ) {
                    Ok(page) => page,
                    Err(error) => {
                        tracing::warn!(%error, "gateway service target page is invalid");
                        continue;
                    }
                };
                let targets = Arc::clone(&service_targets);
                target_scan = Some(Box::pin(async move {
                    targets.list_service_targets(page).await
                }));
            }
            result = async {
                match target_scan.as_mut() {
                    Some(scan) => Some(scan.await),
                    None => std::future::pending().await,
                }
            }, if target_scan.is_some() => {
                target_scan = None;
                match result {
                    Some(Ok(page)) => {
                        target_scan_after = page.next_after;
                        let cleanup_queued = schedule_one_cleanup_retry(
                            &mut service_supervisor,
                            &mut tracked_jobs,
                            &mut cleanup_retry_cursor,
                            service_claim_resolution.as_ref(),
                            service_expired_claim_recovery.as_ref(),
                        );
                        reconcile_service_target_page(
                            &mut service_supervisor,
                            &mut tracked_jobs,
                            page,
                            !cleanup_queued,
                        );
                    }
                    Some(Err(error)) => {
                        tracing::warn!(%error, "gateway service target scan failed");
                        target_scan_after = None;
                    }
                    None => unreachable!("target scan branch is enabled only with a scan"),
                }
            }
            _ = target_refresh_interval.tick(),
                if target_refresh.is_none() && !tracked_jobs.is_empty() =>
            {
                if let Some(job_id) = next_tracked_job_id(&tracked_jobs, target_refresh_cursor)
                {
                    target_refresh_cursor = Some(job_id);
                    let job = tracked_jobs
                        .get(&job_id)
                        .expect("refresh cursor points at tracked job");
                    let gateway_id = job.gateway_id;
                    let revision_id = job.revision_id;
                    let targets = Arc::clone(&service_targets);
                    target_refresh = Some(Box::pin(async move {
                        let result = tokio::time::timeout(
                            SERVICE_TARGET_REFRESH_TIMEOUT,
                            targets.get_service_target(gateway_id, revision_id),
                        )
                        .await
                        .unwrap_or(Err(gateway_edge::GatewayEdgeError::Unavailable));
                        (job_id, gateway_id, revision_id, result)
                    }));
                }
            }
            result = async {
                match target_refresh.as_mut() {
                    Some(refresh) => Some(refresh.await),
                    None => std::future::pending().await,
                }
            }, if target_refresh.is_some() => {
                target_refresh = None;
                if let Some((job_id, gateway_id, revision_id, result)) = result
                    && let Some(job) = tracked_jobs.get_mut(&job_id)
                    && job.gateway_id == gateway_id
                    && job.revision_id == revision_id
                {
                    match result {
                        Ok(Some(target)) => {
                            reconcile_tracked_service_job(job, &target);
                        }
                        Ok(None) => {
                            job.handle.cancel();
                            job.retirement_requested = true;
                        }
                        Err(error) => {
                            tracing::debug!(
                                job_id = %job_id,
                                gateway_id = %gateway_id,
                                revision_id = %revision_id,
                                %error,
                                "gateway service target refresh is unavailable"
                            );
                        }
                    }
                }
            }
            event = async {
                if service_supervisor.has_pending_jobs() {
                    service_supervisor.poll().await
                } else {
                    std::future::pending().await
                }
            }, if service_supervisor.has_pending_jobs() => {
                if let Some(event) = event {
                    tracing::debug!(
                        job_id = %event.job_id,
                        status = ?event.status,
                        capacity_released = event.capacity_released,
                        "gateway service supervisor job completed"
                    );
                    if event.capacity_released {
                        tracked_jobs.remove(&event.job_id);
                    } else if let Some(job) = tracked_jobs.get_mut(&event.job_id) {
                        let now = Instant::now();
                        if job.cleanup_retry_attempted {
                            job.cleanup_retry_due = now.checked_add(job.cleanup_retry_backoff);
                            job.cleanup_retry_backoff = (job.cleanup_retry_backoff * 2)
                                .min(SERVICE_CLEANUP_RETRY_MAX_BACKOFF);
                        } else {
                            job.cleanup_retry_attempted = true;
                            job.cleanup_retry_due = Some(now);
                        }
                    }
                }
            }
            _ = cleanup_retry_interval.tick() => {
                let _ = schedule_one_cleanup_retry(
                    &mut service_supervisor,
                    &mut tracked_jobs,
                    &mut cleanup_retry_cursor,
                    service_claim_resolution.as_ref(),
                    service_expired_claim_recovery.as_ref(),
                );
            }
        }
    }
}

async fn reconcile_gateway_once(
    authority: PostgresGatewayEdgeAuthority,
    provider: Arc<dyn GatewayProvider>,
    recover: bool,
) {
    let desired = match authority.desired_configuration().await {
        Ok(desired) => desired,
        Err(error) => {
            tracing::warn!(%error, "gateway desired-route reconstruction failed");
            return;
        }
    };
    let result = if recover {
        provider.recover(&desired).await
    } else {
        provider.reconcile(&desired).await
    };
    if let Err(error) = result {
        tracing::warn!(%error, recovery = recover, "gateway Caddy reconciliation failed");
    }
}

/// Reconciles one bounded page of durable service targets.
///
/// The active service is restored first. A desired replacement is admitted
/// only after that active service reports `Ready`; the supervisor owns durable
/// promotion and the coordinator owns exact drain counts.
fn reconcile_service_target_page(
    supervisor: &mut GatewayServiceSupervisor,
    tracked_jobs: &mut HashMap<Uuid, TrackedServiceJob>,
    page: GatewayServiceTargetPageResult,
    allow_startups: bool,
) {
    for target in page.targets {
        if target.lifecycle != "enabled" {
            continue;
        }
        let active_is_eligible = target
            .active_service_revision
            .as_ref()
            .is_some_and(|active| {
                target.active_revision_id == Some(active.revision_id) && active.publication_eligible
            });
        if allow_startups
            && let Some(active) = target.active_service_revision.as_ref()
            && active_is_eligible
        {
            start_service_job(
                supervisor,
                tracked_jobs,
                GatewayServiceStartupRequest {
                    gateway_id: target.gateway_id,
                    revision_id: active.revision_id,
                    intent: GatewayServiceStartupIntent::RestoreActive,
                },
            );
        }
        let Some(desired) = target.desired_service_revision.as_ref() else {
            continue;
        };
        if !desired.publication_eligible || target.active_revision_id == Some(desired.revision_id) {
            continue;
        }
        let active_ready = active_is_eligible
            && target
                .active_service_revision
                .as_ref()
                .and_then(|active| tracked_job(tracked_jobs, target.gateway_id, active.revision_id))
                .is_some_and(|job| {
                    *job.handle.subscribe().borrow() == GatewayServiceSupervisorJobStatus::Ready
                });
        let revoked_active_settled = !active_is_eligible
            && target
                .active_service_revision
                .as_ref()
                .is_none_or(|active| {
                    tracked_job(tracked_jobs, target.gateway_id, active.revision_id).is_none()
                });
        let gateway_job_count = tracked_jobs
            .values()
            .filter(|job| job.gateway_id == target.gateway_id)
            .count();
        if allow_startups
            && (active_ready || target.active_service_revision.is_none() || revoked_active_settled)
            && gateway_job_count < 2
        {
            start_service_job(
                supervisor,
                tracked_jobs,
                GatewayServiceStartupRequest {
                    gateway_id: target.gateway_id,
                    revision_id: desired.revision_id,
                    intent: GatewayServiceStartupIntent::ActivateDesired,
                },
            );
        }
    }
}

fn schedule_one_cleanup_retry(
    supervisor: &mut GatewayServiceSupervisor,
    tracked_jobs: &mut HashMap<Uuid, TrackedServiceJob>,
    cursor: &mut Option<Uuid>,
    claim_resolution: Option<&Arc<dyn GatewayServiceClaimResolutionStore>>,
    expired_claim_recovery: Option<&Arc<dyn GatewayServiceExpiredClaimRecovery>>,
) -> bool {
    let pending = tracked_jobs
        .values()
        .filter(|job| {
            *job.handle.subscribe().borrow() == GatewayServiceSupervisorJobStatus::CleanupPending
        })
        .count();
    if pending >= 2 {
        return false;
    }

    let now = Instant::now();
    let mut ids = tracked_jobs.keys().copied().collect::<Vec<_>>();
    ids.sort_unstable();
    let Some(job_id) = cursor
        .and_then(|last| ids.iter().copied().find(|id| *id > last))
        .or_else(|| ids.first().copied())
    else {
        return false;
    };
    let ordered = ids
        .iter()
        .copied()
        .cycle()
        .skip_while(|id| *id != job_id)
        .take(ids.len())
        .collect::<Vec<_>>();
    for candidate in ordered {
        let Some(job) = tracked_jobs.get_mut(&candidate) else {
            continue;
        };
        let due = job
            .cleanup_retry_due
            .is_some_and(|deadline| deadline <= now);
        if !due {
            continue;
        }
        *cursor = Some(candidate);
        let retry_result = if let Some(recovery) = expired_claim_recovery {
            supervisor.retry_cleanup_with_recovery(candidate, Arc::clone(recovery))
        } else {
            supervisor.retry_cleanup(candidate)
        };
        match retry_result {
            Ok(()) => {
                job.cleanup_retry_due = None;
                return true;
            }
            Err(gateway_edge::GatewayServiceSupervisorError::RetryNotEligible) => {
                // A claim acknowledgement may have been lost before the
                // coordinator returned. Resolve it behind the serialized
                // gateway barrier instead of silently stranding its capacity.
                let resolved = claim_resolution.is_some_and(|resolver| {
                    match supervisor.reconcile_claim(candidate, Arc::clone(resolver)) {
                        Ok(()) => true,
                        Err(gateway_edge::GatewayServiceSupervisorError::RetryAlreadyInFlight) => {
                            job.cleanup_retry_due = now.checked_add(job.cleanup_retry_backoff);
                            false
                        }
                        Err(
                            gateway_edge::GatewayServiceSupervisorError::RetryNotFound
                            | gateway_edge::GatewayServiceSupervisorError::RetryNotEligible,
                        ) => {
                            job.cleanup_retry_due = None;
                            false
                        }
                        Err(error) => {
                            tracing::debug!(
                                job_id = %candidate,
                                %error,
                                "gateway service claim reconciliation was not scheduled"
                            );
                            job.cleanup_retry_due = now.checked_add(job.cleanup_retry_backoff);
                            false
                        }
                    }
                });
                if resolved {
                    job.cleanup_retry_due = None;
                    return true;
                }
                if claim_resolution.is_none() {
                    job.cleanup_retry_due = None;
                }
            }
            Err(gateway_edge::GatewayServiceSupervisorError::RetryNotFound) => {
                job.cleanup_retry_due = None;
            }
            Err(gateway_edge::GatewayServiceSupervisorError::RetryAlreadyInFlight) => {
                job.cleanup_retry_due = now.checked_add(job.cleanup_retry_backoff);
            }
            Err(error) => {
                tracing::debug!(job_id = %candidate, %error, "gateway service cleanup retry was not scheduled");
                job.cleanup_retry_due = now.checked_add(job.cleanup_retry_backoff);
            }
        }
    }
    false
}

fn start_service_job(
    supervisor: &mut GatewayServiceSupervisor,
    tracked_jobs: &mut HashMap<Uuid, TrackedServiceJob>,
    request: GatewayServiceStartupRequest,
) {
    if tracked_job(tracked_jobs, request.gateway_id, request.revision_id).is_some() {
        return;
    }
    match supervisor.start(request) {
        Ok(handle) => {
            let job_id = handle.job_id();
            tracked_jobs.insert(
                job_id,
                TrackedServiceJob {
                    gateway_id: request.gateway_id,
                    revision_id: request.revision_id,
                    handle,
                    retirement_requested: false,
                    cleanup_retry_due: None,
                    cleanup_retry_backoff: SERVICE_CLEANUP_RETRY_INITIAL_BACKOFF,
                    cleanup_retry_attempted: false,
                },
            );
        }
        Err(error) => {
            tracing::debug!(
                gateway_id = %request.gateway_id,
                revision_id = %request.revision_id,
                %error,
                "gateway service startup was not admitted"
            );
        }
    }
}

fn tracked_job(
    tracked_jobs: &HashMap<Uuid, TrackedServiceJob>,
    gateway_id: Uuid,
    revision_id: Uuid,
) -> Option<&TrackedServiceJob> {
    tracked_jobs
        .values()
        .find(|job| job.gateway_id == gateway_id && job.revision_id == revision_id)
}

fn next_tracked_job_id(
    tracked_jobs: &HashMap<Uuid, TrackedServiceJob>,
    cursor: Option<Uuid>,
) -> Option<Uuid> {
    let mut ids = tracked_jobs.keys().copied().collect::<Vec<_>>();
    ids.sort_unstable();
    cursor
        .and_then(|cursor| ids.iter().copied().find(|id| *id > cursor))
        .or_else(|| ids.first().copied())
}

fn reconcile_tracked_service_job(
    job: &mut TrackedServiceJob,
    target: &gateway_edge::GatewayServiceOwnedTarget,
) {
    let candidate_is_still_desired = target.desired_service_revision_id == Some(job.revision_id);
    let target_is_current = target.active_revision_id == Some(job.revision_id);
    if !target.revision.publication_eligible {
        job.handle.cancel();
        job.retirement_requested = true;
        return;
    }
    if job.retirement_requested {
        let status = *job.handle.subscribe().borrow();
        if target.lifecycle == "enabled"
            && target_is_current
            && status == GatewayServiceSupervisorJobStatus::Ready
        {
            // A stale paused/superseded read may have sent a drain request
            // just before the gateway became current again. Clear that
            // request marker while the worker is still Ready so a later
            // exact retirement read can retry after a durable Conflict.
            job.retirement_requested = false;
        } else if status == GatewayServiceSupervisorJobStatus::Ready {
            // `mark_draining` can reject a stale target read after a
            // concurrent gateway transition. The next paced exact refresh
            // must be able to submit the request again.
            job.handle.request_drain();
        }
        return;
    }
    if target.lifecycle != "enabled" {
        retire_tracked_service_job(job);
        return;
    }
    if !target_is_current && candidate_is_still_desired {
        // A desired candidate remains available until its own coordinator
        // promotes it; it is not obsolete merely because A still serves.
        return;
    }
    if !target_is_current {
        retire_tracked_service_job(job);
    }
}

fn retire_tracked_service_job(job: &mut TrackedServiceJob) {
    if job.retirement_requested {
        return;
    }
    if *job.handle.subscribe().borrow() == GatewayServiceSupervisorJobStatus::Ready {
        job.handle.request_drain();
    } else {
        job.handle.cancel();
    }
    job.retirement_requested = true;
}

async fn reap_failed_start(tasks: Vec<JoinHandle<Result<(), String>>>) {
    for task in tasks {
        task.abort();
        drop(task.await);
    }
}

async fn oci_builder_loop(workers: Arc<OciBuilderWorkers>, cancellation: CancellationToken) {
    let mut interval = tokio::time::interval(workers.poll_interval);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            () = cancellation.cancelled() => break,
            _ = interval.tick() => {
                oci_builder_pass(&workers).await;
            }
        }
    }
}

async fn registry_reconciliation_loop(
    reconciler: RegistryReconciler<
        PostgresRegistryReconciliation,
        PostgresRegistryReconciliation,
        ZotHttpRegistry<InternalRegistryTokens>,
    >,
    executor: PostgresRegistryReconciliation,
    lease: Duration,
    poll_interval: Duration,
    cancellation: CancellationToken,
) {
    let mut interval = tokio::time::interval(poll_interval);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            () = cancellation.cancelled() => break,
            _ = interval.tick() => {
                if let Err(error) = reconciler.process_next_and_apply(lease, &executor).await {
                    tracing::warn!(%error, "registry notification reconciliation pass failed");
                }
                if let Err(error) = reconciler.reconcile_all_and_apply(&executor).await {
                    // Zot availability is intentionally not forge readiness:
                    // approved consumers remain fail-closed from durable state.
                    tracing::warn!(%error, "registry authoritative reconciliation pass failed");
                }
            }
        }
    }
}

// The two worker outcomes and their independently durable manifest update are
// intentionally explicit; Clippy counts the async/logging expansion as well.
#[allow(clippy::cognitive_complexity)]
async fn oci_builder_pass(workers: &OciBuilderWorkers) {
    if let Err(error) = workers.preparation.run_once().await {
        tracing::warn!(%error, "OCI preparation worker pass failed");
    }
    let materialization_changed = match workers.materialization.run_once().await {
        Ok(changed) => changed,
        Err(error) => {
            tracing::warn!(%error, "OCI rootfs materialization worker pass failed");
            false
        }
    };
    if let Err(error) =
        write_oci_manifest_if_dirty(&workers.manifest_dirty, materialization_changed, || {
            workers.materialization.write_manifest(&workers.manifest)
        })
        .await
    {
        tracing::warn!(%error, "OCI builder root manifest update failed");
    }
    // Refresh from durable roots on every pass. A successful materialization
    // followed by a transient manifest or cache error must be retried even
    // when the next claim pass has no new materialization job.
    if let Err(error) = workers.refresh_image_filesystems().await {
        tracing::warn!(%error, "OCI builder image cache refresh failed");
    }
}

async fn write_oci_manifest_if_dirty<F, Fut>(
    dirty: &AtomicBool,
    materialization_changed: bool,
    write_manifest: F,
) -> Result<(), OciWorkerError>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<(), OciWorkerError>>,
{
    if materialization_changed {
        dirty.store(true, Ordering::Release);
    }
    if !dirty.load(Ordering::Acquire) {
        return Ok(());
    }
    write_manifest().await?;
    dirty.store(false, Ordering::Release);
    Ok(())
}

struct OutboxWorker {
    forge_publisher: ForgeNatsOutboxPublisher,
    release_publisher: ReleaseOutboxPublisher,
    review_publisher: ReviewOutboxPublisher,
    event_publisher: event_adapter::EventPublisher,
    mailbox_publisher: MailboxOutboxPublisher,
    forge: Arc<PgForgeRepository>,
    poll_interval: Duration,
    batch_size: i64,
}

impl OutboxWorker {
    // Rust 1.85 Clippy incorrectly reports Tokio's private select expansion as
    // redundant public crate visibility.
    #[allow(clippy::redundant_pub_crate)]
    // The explicit worker fan-out keeps each durable outbox and its failure
    // policy visible in one supervised loop.
    #[allow(clippy::cognitive_complexity)]
    async fn run(self, cancellation: CancellationToken, ready: oneshot::Sender<()>) {
        let mut interval = tokio::time::interval(self.poll_interval);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        if ready.send(()).is_err() {
            return;
        }
        loop {
            tokio::select! {
                () = cancellation.cancelled() => break,
                _ = interval.tick() => {
                    if let Err(error) = self
                        .forge_publisher
                        .publish_pending(self.forge.as_ref(), self.batch_size)
                        .await
                    {
                        tracing::warn!(%error, "forge outbox publication pass failed");
                    }
                    if let Err(error) = self
                        .release_publisher
                        .publish_pending(self.batch_size)
                        .await
                    {
                        tracing::warn!(%error, "release outbox publication pass failed");
                    }
                    if let Err(error) = self
                        .review_publisher
                        .publish_pending(self.batch_size)
                        .await
                    {
                        tracing::warn!(%error, "review outbox publication pass failed");
                    }
                    if let Err(error) = self.event_publisher.publish_pending(self.batch_size).await {
                        tracing::warn!(%error, "product-event outbox publication pass failed");
                    }
                    if let Err(error) = self.mailbox_publisher.publish_pending(self.batch_size).await {
                        tracing::warn!(%error, "mailbox outbox publication pass failed");
                    }
                }
            }
        }
    }
}

async fn secret_revocation_loop(
    pool: PgPool,
    orchestrator: Arc<RunOrchestrator>,
    poll_interval: Duration,
    cancellation: CancellationToken,
    ready: oneshot::Sender<()>,
) -> Result<(), String> {
    reconcile_revoked_raw_runs(&pool, orchestrator.as_ref()).await?;
    if ready.send(()).is_err() {
        return Ok(());
    }
    loop {
        tokio::select! {
            () = cancellation.cancelled() => return Ok(()),
            () = tokio::time::sleep(poll_interval) => {
                reconcile_revoked_raw_runs(&pool, orchestrator.as_ref()).await?;
            }
        }
    }
}

async fn mailbox_recovery_loop(
    store: Arc<dyn MailboxDispatchStore>,
    poll_interval: Duration,
    cancellation: CancellationToken,
    ready: oneshot::Sender<()>,
) -> Result<(), String> {
    store.recover().await.map_err(|error| error.to_string())?;
    store
        .cleanup_expired_payloads(100)
        .await
        .map_err(|error| error.to_string())?;
    if ready.send(()).is_err() {
        return Ok(());
    }
    loop {
        tokio::select! {
            () = cancellation.cancelled() => return Ok(()),
            () = tokio::time::sleep(poll_interval) => {
                if let Err(error) = store.recover().await {
                    tracing::warn!(%error, "mailbox recovery pass failed");
                }
                if let Err(error) = store.cleanup_expired_payloads(100).await {
                    tracing::warn!(%error, "mailbox payload retention pass failed");
                }
            }
        }
    }
}

async fn reconcile_revoked_raw_runs(
    pool: &PgPool,
    canceller: &(impl RevokedRawRunCanceller + ?Sized),
) -> Result<usize, String> {
    let run_ids = control_plane_postgres::revoked_raw_run_ids(pool)
        .await
        .map_err(|error| error.to_string())?;
    let mut cancellation_count = 0;
    for run_id in run_ids {
        let run_id = RunId::from_uuid(run_id);
        if canceller.cancel_revoked_raw_run(run_id).await? {
            cancellation_count += 1;
        }
    }
    Ok(cancellation_count)
}

#[async_trait]
trait RevokedRawRunCanceller: Sync {
    async fn cancel_revoked_raw_run(&self, run_id: RunId) -> Result<bool, String>;
}

#[async_trait]
impl RevokedRawRunCanceller for RunOrchestrator {
    async fn cancel_revoked_raw_run(&self, run_id: RunId) -> Result<bool, String> {
        self.cancel_run(&CancelRun {
            command_id: CommandId::new(),
            run_id,
            reason: String::from("raw secret authority was revoked"),
        })
        .await
        .map_err(|error| error.to_string())
    }
}

async fn build_secret_mount_manager(
    pool: PgPool,
    database_url: &str,
    keys: LocalKeyProvider,
    config: EphemeralSecretConfig,
) -> Result<
    (
        Arc<dyn RunSecretManager>,
        Arc<SecretRuntimeService<LocalKeyProvider>>,
        Arc<SecretService<LocalKeyProvider>>,
    ),
    AppError,
> {
    let resolver_pool = connect_control_plane(database_url, 4)
        .await
        .map_err(component("secret resolver PostgreSQL connection"))?;
    let authorizer = Arc::new(authz_postgres::PostgresMelangeAuthorizer);
    let dispatch = Arc::new(SecretService::new(
        pool.clone(),
        EncryptedStore::new(keys.clone()),
        authorizer.clone(),
    ));
    let runtime = Arc::new(SecretRuntimeService::new(
        pool.clone(),
        resolver_pool,
        EncryptedStore::new(keys),
        authorizer,
    ));
    let manager = initialize_manager(
        pool,
        dispatch.as_ref().clone(),
        runtime.as_ref().clone(),
        config,
    )
    .map_err(component("secret mount initialization"))?;
    Ok((Arc::new(manager), runtime, dispatch))
}

struct UpdateRunCompletion {
    pool: PgPool,
    releases: Arc<ReleaseService>,
    admission_cursor: Mutex<Option<(OffsetDateTime, Uuid)>>,
}

impl UpdateRunCompletion {
    async fn next_pending_admissions(
        &self,
    ) -> Result<Vec<control_plane_postgres::PendingUpdateAdmission>, RunCompletionError> {
        let mut cursor = self.admission_cursor.lock().await;
        let admissions = pending_update_admissions(&self.pool, *cursor)
            .await
            .map_err(completion_error)?;
        if admissions.is_empty() {
            *cursor = None;
            return Ok(Vec::new());
        }
        *cursor = admissions
            .last()
            .map(|admission| (admission.created_at, admission.update_id));
        drop(cursor);
        Ok(admissions)
    }

    async fn resume_pending_admission(
        &self,
        admission: control_plane_postgres::PendingUpdateAdmission,
    ) -> Result<bool, RunCompletionError> {
        let identity = AuthenticatedIdentity::new(
            UserId::from_uuid(admission.actor_id),
            "hephaestus-update-recovery",
            "durable-update-recovery",
            serde_json::json!({}),
            RequestId::new(),
        );
        let generation = admission.generation.to_be_bytes();
        let hook_run_id =
            deterministic_update_hook_run_id(admission.update_id, admission.generation);
        let command_key = ReleaseCommandKey::derive(
            "begin_update_hook.recovery",
            &[admission.update_id.as_bytes(), &generation],
        );
        let result = self
            .releases
            .begin_update_hook(
                &identity,
                BeginUpdateHook {
                    command_key,
                    update_id: release_domain::AgentUpdateId::from_uuid(admission.update_id),
                    hook_run_id,
                },
            )
            .await;
        match result {
            Ok(()) => {
                #[cfg(feature = "test-fixtures")]
                application::commands::notify_reconciler_update_admission(admission.update_id);
                Ok(true)
            }
            Err(error) => {
                classify_pending_admission_error(admission.update_id, admission.actor_id, error)
            }
        }
    }

    async fn resume_pending_admissions(&self) -> Result<usize, RunCompletionError> {
        let admissions = self.next_pending_admissions().await?;
        let mut resumed = 0;
        for admission in admissions {
            if self.resume_pending_admission(admission).await? {
                resumed += 1;
            }
        }
        Ok(resumed)
    }

    async fn apply(&self, run: &Run) -> Result<bool, RunCompletionError> {
        if run.kind != RunKind::Update {
            return Ok(false);
        }
        let is_update_hook = is_update_hook_run(&self.pool, run.id.as_uuid())
            .await
            .map_err(completion_error)?;
        if !is_update_hook {
            return Ok(false);
        }
        self.releases
            .reconcile_update_run(run.id)
            .await
            .map_err(completion_error)?;
        Ok(true)
    }
}

fn classify_pending_admission_error(
    update_id: Uuid,
    actor_id: Uuid,
    error: ReleaseServiceError,
) -> Result<bool, RunCompletionError> {
    match admission_failure_kind(&error) {
        AdmissionFailureKind::DrainPending => Ok(log_drain_pending(update_id)),
        AdmissionFailureKind::AuthorizationDenied => {
            Ok(log_authorization_denied(update_id, actor_id))
        }
        AdmissionFailureKind::InvalidLifecycle => Ok(log_invalid_lifecycle(update_id)),
        AdmissionFailureKind::GenerationRace => Ok(log_generation_race(update_id)),
        AdmissionFailureKind::Other => Err(completion_error(error)),
    }
}

fn log_drain_pending(update_id: Uuid) -> bool {
    tracing::debug!(
        update_id = %update_id,
        "durable update remains fenced until normal work cleans up"
    );
    false
}

fn log_authorization_denied(update_id: Uuid, actor_id: Uuid) -> bool {
    tracing::error!(
        update_id = %update_id,
        actor_id = %actor_id,
        "durable update admission authorization was denied; leaving it fenced"
    );
    false
}

fn log_invalid_lifecycle(update_id: Uuid) -> bool {
    tracing::warn!(
        update_id = %update_id,
        "durable update admission reached an inspectable lifecycle boundary"
    );
    false
}

fn log_generation_race(update_id: Uuid) -> bool {
    // A retry can race the generation snapshot with the previous hook's
    // cleanup. The row remains draining and the next bounded reconciliation
    // observes the committed run count and derives the next identity.
    tracing::debug!(
        update_id = %update_id,
        "durable update admission generation raced; deferring"
    );
    false
}

enum AdmissionFailureKind {
    DrainPending,
    AuthorizationDenied,
    InvalidLifecycle,
    GenerationRace,
    Other,
}

const fn admission_failure_kind(error: &ReleaseServiceError) -> AdmissionFailureKind {
    if matches!(error, ReleaseServiceError::UpdateDrainPending) {
        return AdmissionFailureKind::DrainPending;
    }
    if matches!(error, ReleaseServiceError::AuthorizationDenied) {
        return AdmissionFailureKind::AuthorizationDenied;
    }
    if matches!(error, ReleaseServiceError::InvalidUpdateLifecycle) {
        return AdmissionFailureKind::InvalidLifecycle;
    }
    if matches!(error, ReleaseServiceError::UpdateAdmissionGenerationRace) {
        return AdmissionFailureKind::GenerationRace;
    }
    AdmissionFailureKind::Other
}

#[async_trait]
impl RunCompletionObserver for UpdateRunCompletion {
    async fn after_cleanup(&self, run: &Run) -> Result<(), RunCompletionError> {
        self.apply(run).await?;
        if run.kind == RunKind::Normal {
            self.resume_pending_admissions().await?;
        }
        Ok(())
    }

    async fn recover(&self) -> Result<usize, RunCompletionError> {
        let run_ids = recoverable_update_hook_run_ids(&self.pool)
            .await
            .map_err(completion_error)?;
        let mut recovered = 0;
        for run_id in run_ids {
            self.releases
                .reconcile_update_run(RunId::from_uuid(run_id))
                .await
                .map_err(completion_error)?;
            recovered += 1;
        }
        recovered += self.resume_pending_admissions().await?;
        Ok(recovered)
    }
}

/// Derives a new hook run identity for each durable attempt generation.
///
/// The generation is persisted indirectly by the update-run history, so a
/// recovered retry cannot collide with the run that preceded it.
fn deterministic_update_hook_run_id(update_id: Uuid, generation: i64) -> RunId {
    let mut digest = Sha256::new();
    digest.update(b"hephaestus:update-hook-run-v2\0");
    digest.update(update_id.as_bytes());
    digest.update(generation.to_be_bytes());
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest.finalize()[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    RunId::from_uuid(Uuid::from_bytes(bytes))
}

/// Reconciles accepted and retry-scheduled updates after their triggering
/// normal-run cleanup callback has already fired.
async fn update_admission_reconciliation_loop(
    observer: Arc<UpdateRunCompletion>,
    cancellation: CancellationToken,
    configured_interval: Duration,
    ready: oneshot::Sender<()>,
) {
    if ready.send(()).is_err() {
        return;
    }
    let interval = configured_interval.max(Duration::from_secs(1));
    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            () = cancellation.cancelled() => break,
            _ = ticker.tick() => {
                match tokio::time::timeout(
                    Duration::from_secs(1),
                    observer.resume_pending_admissions(),
                )
                .await
                {
                    Ok(Ok(_)) => {}
                    Ok(Err(error)) => {
                        tracing::warn!(%error, "update admission reconciliation deferred");
                    }
                    Err(_) => {
                        tracing::warn!("update admission reconciliation timed out");
                    }
                }
            }
        }
    }
}

#[derive(Deserialize)]
struct BuildRequestedPayload {
    build_request_id: Uuid,
}

async fn build_loop(
    consumer: async_nats::jetstream::consumer::PullConsumer,
    executor: Arc<BuildExecutor>,
    concurrency: usize,
    cancellation: CancellationToken,
    ready: oneshot::Sender<()>,
) -> Result<(), String> {
    let mut messages = consumer
        .messages()
        .await
        .map_err(|error| error.to_string())?;
    let permits = Arc::new(Semaphore::new(concurrency));
    let mut builds = JoinSet::new();
    if ready.send(()).is_err() {
        return Ok(());
    }
    loop {
        tokio::select! {
            () = cancellation.cancelled() => break,
            delivery = messages.next() => {
                let Some(delivery) = delivery else {
                    return Err(String::from("build command stream ended"));
                };
                let message = delivery.map_err(|error| error.to_string())?;
                let permit = Arc::clone(&permits)
                    .acquire_owned()
                    .await
                    .map_err(|error| error.to_string())?;
                let executor = Arc::clone(&executor);
                builds.spawn(async move {
                    let _permit = permit;
                    if let Err(error) = handle_build_message(&executor, &message).await {
                        tracing::warn!(%error, "build command handling failed");
                    }
                });
            }
            result = builds.join_next(), if !builds.is_empty() => {
                if let Some(Err(error)) = result {
                    tracing::warn!(%error, "build command task panicked");
                }
            }
        }
    }
    while let Some(result) = builds.join_next().await {
        if let Err(error) = result {
            tracing::warn!(%error, "build command task panicked while draining");
        }
    }
    Ok(())
}

async fn handle_build_message(
    executor: &BuildExecutor,
    message: &async_nats::jetstream::Message,
) -> Result<(), String> {
    let retry = message.message.subject.as_str() == BUILD_RETRY_REQUESTED_SUBJECT;
    let verify = message.message.subject.as_str() == BUILD_VERIFY_REQUESTED_SUBJECT;
    if message.message.subject.as_str() != BUILD_REQUESTED_SUBJECT && !retry && !verify {
        message
            .ack_with(async_nats::jetstream::AckKind::Term)
            .await
            .map_err(|error| error.to_string())?;
        return Err(String::from("unknown build command subject"));
    }
    let payload: BuildRequestedPayload = match serde_json::from_slice(&message.payload) {
        Ok(payload) => payload,
        Err(error) => {
            message
                .ack_with(async_nats::jetstream::AckKind::Term)
                .await
                .map_err(|ack_error| ack_error.to_string())?;
            return Err(error.to_string());
        }
    };
    let operation = async {
        if verify {
            executor
                .verify(BuildRequestId::from_uuid(payload.build_request_id))
                .await
        } else if retry {
            executor
                .retry(BuildRequestId::from_uuid(payload.build_request_id))
                .await
                .map(|_| ())
        } else {
            executor
                .execute(BuildRequestId::from_uuid(payload.build_request_id))
                .await
                .map(|_| ())
        }
    };
    tokio::pin!(operation);
    let mut progress = tokio::time::interval(Duration::from_secs(10));
    progress.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            result = &mut operation => {
                match result {
                    Ok(()) => {
                        message.double_ack().await.map_err(|error| error.to_string())?;
                        return Ok(());
                    }
                    Err(error) if build_delivery_requires_redelivery(&error) => {
                        return Err(error.to_string());
                    }
                    Err(error) => {
                        message.double_ack().await.map_err(|ack_error| ack_error.to_string())?;
                        return Err(error.to_string());
                    }
                }
            }
            _ = progress.tick() => {
                if let Err(error) = message
                    .ack_with(async_nats::jetstream::AckKind::Progress)
                    .await
                {
                    tracing::warn!(%error, "failed to acknowledge build progress");
                }
            }
        }
    }
}

const fn build_delivery_requires_redelivery(error: &BuildExecutionError) -> bool {
    matches!(
        error,
        BuildExecutionError::Database(_)
            | BuildExecutionError::Release
            | BuildExecutionError::ImageUnavailable
            | BuildExecutionError::VmCleanup
    )
}

fn completion_error(error: impl std::fmt::Display) -> RunCompletionError {
    tracing::error!(%error, "update-run completion processing failed");
    RunCompletionError::redacted("durable update result processing failed")
}

// Rust 1.85 Clippy incorrectly reports Tokio's private select expansion as
// redundant public crate visibility.
#[allow(clippy::redundant_pub_crate)]
async fn mailbox_command_loop(
    consumer: async_nats::jetstream::consumer::PullConsumer,
    handler: NatsMailboxCommandHandler,
    concurrency: usize,
    cancellation: CancellationToken,
    ready: oneshot::Sender<()>,
) -> Result<(), String> {
    let mut messages = consumer
        .messages()
        .await
        .map_err(|error| error.to_string())?;
    let permits = Arc::new(Semaphore::new(concurrency));
    let mut commands = JoinSet::new();
    if ready.send(()).is_err() {
        return Ok(());
    }
    loop {
        tokio::select! {
            () = cancellation.cancelled() => break,
            delivery = messages.next() => {
                let Some(delivery) = delivery else {
                    return Err(String::from("mailbox command stream ended"));
                };
                let message = delivery.map_err(|error| error.to_string())?;
                let permit = Arc::clone(&permits)
                    .acquire_owned()
                    .await
                    .map_err(|error| error.to_string())?;
                let handler = handler.clone();
                commands.spawn(async move {
                    let _permit = permit;
                    if let Err(error) = handler.handle(&message).await {
                        tracing::warn!(%error, "mailbox command was not acknowledged");
                    }
                });
            }
            result = commands.join_next(), if !commands.is_empty() => {
                if let Some(Err(error)) = result {
                    tracing::warn!(%error, "mailbox command task panicked");
                }
            }
        }
    }
    while let Some(result) = commands.join_next().await {
        if let Err(error) = result {
            tracing::warn!(%error, "mailbox command task panicked while draining");
        }
    }
    Ok(())
}

// Rust 1.85 Clippy incorrectly reports Tokio's private select expansion as
// redundant public crate visibility.
#[allow(clippy::redundant_pub_crate)]
async fn command_loop(
    consumer: async_nats::jetstream::consumer::PullConsumer,
    handler: NatsCommandHandler,
    control_handler: NatsControlHandler,
    concurrency: usize,
    cancellation: CancellationToken,
    ready: oneshot::Sender<()>,
) -> Result<(), String> {
    let mut messages = consumer
        .messages()
        .await
        .map_err(|error| error.to_string())?;
    let permits = Arc::new(Semaphore::new(concurrency));
    let mut commands = JoinSet::new();
    if ready.send(()).is_err() {
        return Ok(());
    }
    loop {
        tokio::select! {
            () = cancellation.cancelled() => break,
            delivery = messages.next() => {
                let Some(delivery) = delivery else {
                    return Err(String::from("run command stream ended"));
                };
                let message = delivery.map_err(|error| error.to_string())?;
                let permit = Arc::clone(&permits)
                    .acquire_owned()
                    .await
                    .map_err(|error| error.to_string())?;
                let handler = handler.clone();
                let control_handler = control_handler.clone();
                commands.spawn(async move {
                    let _permit = permit;
                    if message.message.subject.as_str() == CONTROL_EXECUTE_SUBJECT {
                        if let Err(error) = control_handler.handle(&message).await {
                            tracing::warn!(%error, "control command was not acknowledged");
                        }
                    } else if let Err(error) = handler.handle(&message).await {
                        tracing::warn!(%error, "run command was not acknowledged");
                    }
                });
            }
            result = commands.join_next(), if !commands.is_empty() => {
                if let Some(Err(error)) = result {
                    tracing::warn!(%error, "run command task panicked");
                }
            }
        }
    }
    while let Some(result) = commands.join_next().await {
        if let Err(error) = result {
            tracing::warn!(%error, "run command task panicked while draining");
        }
    }
    Ok(())
}

/// Running daemon handle.
pub struct RunningHephaestus {
    http_addr: SocketAddr,
    cancellation: CancellationToken,
    tasks: Vec<JoinHandle<Result<(), String>>>,
    pool: PgPool,
    application_pool: PgPool,
    service_log_pool: PgPool,
    nats_client: async_nats::Client,
    jetstream: async_nats::jetstream::Context,
    forge: Arc<PgForgeRepository>,
    run_repository: Arc<PgRunRepository>,
    mailbox_repository: Arc<PostgresMailboxRepository>,
    review_repository: Arc<PostgresReviewRepository>,
    orchestrator: Arc<RunOrchestrator>,
    outbox_batch_size: i64,
    product_event_cursor_key: [u8; 32],
    shutdown_timeout: Duration,
}

impl RunningHephaestus {
    /// Bound HTTP address after the readiness barrier.
    #[must_use]
    pub const fn http_addr(&self) -> SocketAddr {
        self.http_addr
    }

    /// Returns a retained application-role pool for daemon integration checks.
    #[cfg(feature = "test-fixtures")]
    #[doc(hidden)]
    #[must_use]
    pub fn application_pool_for_test(&self) -> ControlPlanePool {
        self.application_pool.clone()
    }

    /// Returns the dedicated worker pool used by the service-log scheduler.
    #[cfg(feature = "test-fixtures")]
    #[doc(hidden)]
    #[must_use]
    pub fn service_log_pool_for_test(&self) -> ControlPlanePool {
        self.service_log_pool.clone()
    }

    /// Waits for one persisted lifecycle event.
    ///
    /// # Errors
    ///
    /// Returns an error for database failure or timeout.
    pub async fn wait_for_run_event(
        &self,
        run_id: RunId,
        kind: RunEventKind,
        timeout: Duration,
    ) -> Result<(), AppError> {
        let deadline = Instant::now() + timeout;
        loop {
            let exists =
                control_plane_postgres::has_run_event(&self.pool, run_id.as_uuid(), kind.as_str())
                    .await
                    .map_err(component("run event query"))?;
            if exists {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(AppError::Timeout(format!(
                    "run {run_id} did not persist {}",
                    kind.as_str()
                )));
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    /// Stops admission, cancels active runs, drains supervised tasks, and
    /// closes NATS and `PostgreSQL` resources.
    ///
    /// # Errors
    ///
    /// Returns the first task or resource-shutdown failure.
    pub async fn shutdown(mut self) -> Result<(), AppError> {
        self.cancellation.cancel();
        for run in self
            .run_repository
            .recoverable_runs()
            .await
            .map_err(component("recoverable run query"))?
        {
            let command = CancelRun {
                command_id: CommandId::new(),
                run_id: run.id,
                reason: String::from("daemon shutdown"),
            };
            if let Err(error) = self.orchestrator.cancel_run(&command).await {
                tracing::warn!(run_id = %run.id, %error, "active run cancellation failed");
            }
        }

        let deadline = Instant::now() + self.shutdown_timeout;
        let mut first_error = None;
        for mut task in self.tasks.drain(..) {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match tokio::time::timeout(remaining, &mut task).await {
                Ok(Ok(Ok(()))) => {}
                Ok(Ok(Err(error))) => {
                    first_error.get_or_insert(AppError::Task(error));
                }
                Ok(Err(error)) => {
                    first_error.get_or_insert_with(|| AppError::Task(error.to_string()));
                }
                Err(_) => {
                    task.abort();
                    drop(task.await);
                    first_error.get_or_insert_with(|| {
                        AppError::Timeout(String::from("supervised task drain timed out"))
                    });
                }
            }
        }
        if let Err(error) = self.flush_outbox(deadline).await {
            first_error.get_or_insert(error);
        }
        if let Err(error) = self.nats_client.drain().await {
            first_error.get_or_insert_with(|| AppError::Shutdown(error.to_string()));
        }
        self.pool.close().await;
        self.application_pool.close().await;
        self.service_log_pool.close().await;
        first_error.map_or(Ok(()), Err)
    }

    async fn flush_outbox(&self, deadline: Instant) -> Result<(), AppError> {
        let forge_publisher = ForgeNatsOutboxPublisher::new(self.jetstream.clone());
        let release_publisher =
            ReleaseOutboxPublisher::new(self.jetstream.clone(), self.pool.clone());
        let review_publisher = ReviewOutboxPublisher::new(
            self.jetstream.clone(),
            Arc::clone(&self.review_repository) as Arc<dyn review_service::ReviewOutboxStore>,
        );
        let event_publisher = event_adapter::EventPublisher::new(
            self.jetstream.clone(),
            Arc::new(event_postgres::PostgresProductEventOutbox::new(
                self.pool.clone(),
            )),
            self.product_event_cursor_key,
        );
        let mailbox_publisher =
            MailboxOutboxPublisher::new(self.jetstream.clone(), self.mailbox_repository.clone());
        let diagnostics = Arc::new(StdMutex::new(FlushDiagnostics::new(deadline)));
        let result = flush_until_quiescent(deadline, Arc::clone(&diagnostics), || {
            let diagnostics = Arc::clone(&diagnostics);
            let forge_publisher = forge_publisher.clone();
            let release_publisher = release_publisher.clone();
            let review_publisher = review_publisher.clone();
            let event_publisher = event_publisher.clone();
            let mailbox_publisher = mailbox_publisher.clone();
            async move {
                let forge = flush_publisher(
                    &diagnostics,
                    FlushPublisher::Forge,
                    forge_publisher.publish_pending(self.forge.as_ref(), self.outbox_batch_size),
                    "final forge outbox flush",
                )
                .await?;
                let releases = flush_publisher(
                    &diagnostics,
                    FlushPublisher::Release,
                    release_publisher.publish_pending(self.outbox_batch_size),
                    "final release outbox flush",
                )
                .await?;
                let reviews = flush_publisher(
                    &diagnostics,
                    FlushPublisher::Review,
                    review_publisher.publish_pending(self.outbox_batch_size),
                    "final review outbox flush",
                )
                .await?;
                let events = flush_publisher(
                    &diagnostics,
                    FlushPublisher::ProductEvent,
                    event_publisher.publish_pending(self.outbox_batch_size),
                    "final product-event outbox flush",
                )
                .await?;
                let mailboxes = flush_publisher(
                    &diagnostics,
                    FlushPublisher::Mailbox,
                    mailbox_publisher.publish_pending(self.outbox_batch_size),
                    "final mailbox outbox flush",
                )
                .await?;
                Ok::<_, AppError>(
                    forge == 0 && releases == 0 && reviews == 0 && events == 0 && mailboxes == 0,
                )
            }
        })
        .await;
        if result.is_err() {
            diagnostics
                .lock()
                .expect("flush diagnostics mutex is not poisoned")
                .log_failure(deadline);
        }
        result
    }
}

#[derive(Debug, Clone, Copy)]
enum FlushPublisher {
    Forge,
    Release,
    Review,
    ProductEvent,
    Mailbox,
}

impl FlushPublisher {
    const fn name(self) -> &'static str {
        match self {
            Self::Forge => "forge",
            Self::Release => "release",
            Self::Review => "review",
            Self::ProductEvent => "product-event",
            Self::Mailbox => "mailbox",
        }
    }

    const fn index(self) -> usize {
        match self {
            Self::Forge => 0,
            Self::Release => 1,
            Self::Review => 2,
            Self::ProductEvent => 3,
            Self::Mailbox => 4,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct FlushPhase {
    publisher: FlushPublisher,
    started: Instant,
    elapsed: Duration,
    batch_count: Option<usize>,
}

#[derive(Debug)]
struct FlushDiagnostics {
    entered_remaining: Duration,
    passes: u32,
    active: Option<FlushPhase>,
    last_phase: Option<FlushPhase>,
    last_batches: [Option<usize>; 5],
    failure_kind: Option<&'static str>,
    deadline_expired_before_pass: bool,
    deadline_expired_during_pass: bool,
    deadline_expired_during_publisher: bool,
}

impl FlushDiagnostics {
    fn new(deadline: Instant) -> Self {
        Self {
            entered_remaining: deadline.saturating_duration_since(Instant::now()),
            passes: 0,
            active: None,
            last_phase: None,
            last_batches: [None; 5],
            failure_kind: None,
            deadline_expired_before_pass: false,
            deadline_expired_during_pass: false,
            deadline_expired_during_publisher: false,
        }
    }

    fn begin(&mut self, publisher: FlushPublisher) {
        self.active = Some(FlushPhase {
            publisher,
            started: Instant::now(),
            elapsed: Duration::ZERO,
            batch_count: None,
        });
    }

    fn finish(&mut self, batch_count: usize) {
        if let Some(mut phase) = self.active.take() {
            phase.elapsed = phase.started.elapsed();
            phase.batch_count = Some(batch_count);
            self.last_batches[phase.publisher.index()] = Some(batch_count);
            self.last_phase = Some(phase);
        }
    }

    fn finish_error(&mut self) {
        if let Some(mut phase) = self.active.take() {
            phase.elapsed = phase.started.elapsed();
            self.last_phase = Some(phase);
        }
    }

    const fn set_deadline_before_pass(&mut self) {
        self.failure_kind = Some("deadline-before-pass");
        self.deadline_expired_before_pass = true;
    }

    fn log_failure(&self, deadline: Instant) {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let active_publisher = self.active.map(|phase| phase.publisher.name());
        let active_elapsed = self
            .active
            .map(|phase| duration_millis(phase.started.elapsed()));
        let last_publisher = self.last_phase.map(|phase| phase.publisher.name());
        let last_elapsed = self.last_phase.map(|phase| duration_millis(phase.elapsed));
        tracing::warn!(
            entered_remaining_ms = duration_millis(self.entered_remaining),
            remaining_ms = duration_millis(remaining),
            passes = self.passes,
            deadline_expired_before_pass = self.deadline_expired_before_pass,
            deadline_expired_during_pass = self.deadline_expired_during_pass,
            deadline_expired_during_publisher = self.deadline_expired_during_publisher,
            active_publisher = active_publisher.unwrap_or("none"),
            active_elapsed_ms = active_elapsed.unwrap_or(0),
            last_publisher = last_publisher.unwrap_or("none"),
            last_elapsed_ms = last_elapsed.unwrap_or(0),
            forge_last_batch = ?self.last_batches[FlushPublisher::Forge.index()],
            release_last_batch = ?self.last_batches[FlushPublisher::Release.index()],
            review_last_batch = ?self.last_batches[FlushPublisher::Review.index()],
            product_event_last_batch = ?self.last_batches[FlushPublisher::ProductEvent.index()],
            mailbox_last_batch = ?self.last_batches[FlushPublisher::Mailbox.index()],
            failure_kind = self.failure_kind.unwrap_or("unknown"),
            "final outbox flush did not quiesce"
        );
    }
}

fn duration_millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

async fn flush_publisher<Fut, Error>(
    diagnostics: &Arc<StdMutex<FlushDiagnostics>>,
    publisher: FlushPublisher,
    operation: Fut,
    component_name: &'static str,
) -> Result<usize, AppError>
where
    Fut: Future<Output = Result<usize, Error>>,
    Error: std::fmt::Display,
{
    diagnostics
        .lock()
        .expect("flush diagnostics mutex is not poisoned")
        .begin(publisher);
    match operation.await {
        Ok(batch_count) => {
            diagnostics
                .lock()
                .expect("flush diagnostics mutex is not poisoned")
                .finish(batch_count);
            Ok(batch_count)
        }
        Err(error) => {
            let mut diagnostics = diagnostics
                .lock()
                .expect("flush diagnostics mutex is not poisoned");
            diagnostics.failure_kind = Some("publisher-error");
            diagnostics.finish_error();
            drop(diagnostics);
            Err(component(component_name)(error))
        }
    }
}

async fn flush_until_quiescent<F, Fut>(
    deadline: Instant,
    diagnostics: Arc<StdMutex<FlushDiagnostics>>,
    mut pass: F,
) -> Result<(), AppError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<bool, AppError>>,
{
    loop {
        {
            let mut diagnostics = diagnostics
                .lock()
                .expect("flush diagnostics mutex is not poisoned");
            diagnostics.passes = diagnostics.passes.saturating_add(1);
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            diagnostics
                .lock()
                .expect("flush diagnostics mutex is not poisoned")
                .set_deadline_before_pass();
            return Err(AppError::Shutdown(String::from(
                "final outbox flush did not quiesce",
            )));
        }
        let quiescent = tokio::time::timeout(remaining, pass())
            .await
            .map_err(|_| {
                let mut diagnostics = diagnostics
                    .lock()
                    .expect("flush diagnostics mutex is not poisoned");
                diagnostics.failure_kind = Some(if diagnostics.active.is_some() {
                    "deadline-during-publisher"
                } else {
                    "deadline-during-pass"
                });
                diagnostics.deadline_expired_during_pass = true;
                diagnostics.deadline_expired_during_publisher = diagnostics.active.is_some();
                drop(diagnostics);
                AppError::Shutdown(String::from("final outbox flush did not quiesce"))
            })??;
        if quiescent && Instant::now() < deadline {
            return Ok(());
        }
    }
}

/// Persisted run lifecycle event used by operational waits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunEventKind {
    /// The VM reached its running state.
    Running,
    /// The trusted host completed controlled result publication.
    ResultCompleted,
}

impl RunEventKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Running => "run.running",
            Self::ResultCompleted => "result.completed",
        }
    }
}

struct PgRunAuthorityManager {
    repository: PgRuntimeSessionRepository,
    issuer: RuntimeSessionIssuer<PgRuntimeSessionRepository, EncryptedFileHandoffStore>,
    git_issuer: RuntimeGitCredentialIssuer<
        PgRuntimeGitCredentialRepository,
        EncryptedFileRuntimeGitHandoffStore,
    >,
    session_ttl: time::Duration,
}

impl PgRunAuthorityManager {
    fn new(
        pool: PgPool,
        git_repository: PgRuntimeGitCredentialRepository,
        handoff_root: PathBuf,
        handoff_key: [u8; 32],
        session_ttl: Duration,
    ) -> Result<Self, AppError> {
        let repository = PgRuntimeSessionRepository::new(pool);
        let handoff = EncryptedFileHandoffStore::new(handoff_root.clone(), handoff_key)
            .map_err(component("runtime authority handoff"))?;
        let git_handoff = EncryptedFileRuntimeGitHandoffStore::new(handoff_root, handoff_key)
            .map_err(component("runtime Git authority handoff"))?;
        let session_ttl = time::Duration::try_from(session_ttl).map_err(|error| {
            AppError::Configuration(format!("runtime authority session TTL is invalid: {error}"))
        })?;
        Ok(Self {
            issuer: RuntimeSessionIssuer::new(repository.clone(), handoff),
            git_issuer: RuntimeGitCredentialIssuer::new(git_repository, git_handoff),
            repository,
            session_ttl,
        })
    }

    async fn snapshot(
        &self,
        run: &Run,
    ) -> Result<capability_domain::AuthorizationSnapshot, RunAuthorityError> {
        self.repository
            .resolve_snapshot(run, authz_postgres::AUTHORIZATION_MODEL_VERSION)
            .await
            .map_err(authority_error)
    }
}

#[async_trait]
impl RunAuthorityManager for PgRunAuthorityManager {
    async fn prepare(&self, run: &Run) -> Result<PreparedRunAuthority, RunAuthorityError> {
        let snapshot = self.snapshot(run).await?;
        if !self
            .repository
            .live_authorized(run, &snapshot)
            .await
            .map_err(authority_error)?
        {
            return Err(RunAuthorityError::redacted(
                "live capability authority was denied",
            ));
        }
        let session_id = RuntimeSessionId::from_uuid(run.id.as_uuid());
        let existing = self
            .repository
            .find(session_id)
            .await
            .map_err(authority_error)?;
        let issued_at = existing
            .as_ref()
            .map_or_else(OffsetDateTime::now_utc, |session| session.issued_at);
        let expires_at = existing
            .as_ref()
            .map_or(issued_at + self.session_ttl, |session| session.expires_at);
        let identity = RuntimeSessionIdentity::new(
            session_id,
            snapshot.principal(),
            RuntimeInvocation::Run(run.id),
            &snapshot,
            issued_at,
            expires_at,
        )
        .map_err(|_| RunAuthorityError::redacted("runtime identity is invalid"))?;
        let issued = self
            .issuer
            .issue(
                &snapshot,
                &identity,
                run.attachment_id
                    .map(runtime_types::AgentAttachmentId::as_uuid),
                OffsetDateTime::now_utc(),
            )
            .await
            .map_err(authority_error)?;
        let runtime_git = match self
            .git_issuer
            .issue(
                issued.session.id,
                issued.session.generation,
                issued.session.expires_at,
                OffsetDateTime::now_utc(),
            )
            .await
        {
            Ok(issued) => Some(issued),
            Err(RuntimeGitAuthorityError::NotFound) => None,
            Err(error) => return Err(runtime_git_authority_error(error)),
        };
        let mut bootstrap = vm_trait::RuntimeAuthorityBootstrap::new(
            issued.session.id.as_uuid(),
            issued.session.generation.get(),
            *issued.credential.expose(),
        );
        if let Some(runtime_git) = &runtime_git {
            bootstrap = bootstrap.with_runtime_git_credential(*runtime_git.credential.expose());
        }
        Ok(PreparedRunAuthority {
            bootstrap: Some(bootstrap),
        })
    }

    async fn reauthorize(&self, run: &Run) -> Result<(), RunAuthorityError> {
        let snapshot = self.snapshot(run).await?;
        if self
            .repository
            .live_authorized(run, &snapshot)
            .await
            .map_err(authority_error)?
        {
            Ok(())
        } else {
            Err(RunAuthorityError::redacted(
                "live capability authority was revoked",
            ))
        }
    }

    async fn acknowledge(
        &self,
        run: &Run,
        session_id: Uuid,
        generation: u64,
    ) -> Result<(), RunAuthorityError> {
        if session_id != run.id.as_uuid() {
            return Err(RunAuthorityError::redacted(
                "runtime session does not match the exact run",
            ));
        }
        let generation = RuntimeCredentialGeneration::new(generation)
            .map_err(|_| RunAuthorityError::redacted("runtime generation is invalid"))?;
        self.issuer
            .acknowledge(
                RuntimeSessionId::from_uuid(session_id),
                generation,
                OffsetDateTime::now_utc(),
            )
            .await
            .map_err(authority_error)?;
        self.git_issuer
            .acknowledge_or_revoke(RuntimeSessionId::from_uuid(session_id), generation)
            .map_err(runtime_git_authority_error)?;
        Ok(())
    }

    async fn revoke_after_guest(&self, run_id: RunId) -> Result<(), RunAuthorityError> {
        let session_id = RuntimeSessionId::from_uuid(run_id.as_uuid());
        let Some(session) = self
            .repository
            .find(session_id)
            .await
            .map_err(authority_error)?
        else {
            return Ok(());
        };
        if matches!(
            session.status,
            capability_domain::RuntimeSessionStatus::Expired
        ) {
            self.issuer
                .recover_expired(OffsetDateTime::now_utc())
                .await
                .map_err(authority_error)?;
            self.git_issuer
                .recover_expired(OffsetDateTime::now_utc())
                .map_err(runtime_git_authority_error)?;
            return Ok(());
        }
        self.issuer
            .revoke(session_id, OffsetDateTime::now_utc(), "run guest destroyed")
            .await
            .map_err(authority_error)?;
        self.git_issuer
            .acknowledge_or_revoke(session_id, session.generation)
            .map_err(runtime_git_authority_error)?;
        Ok(())
    }

    async fn recover(&self) -> Result<usize, RunAuthorityError> {
        let recovered = self
            .issuer
            .recover_expired(OffsetDateTime::now_utc())
            .await
            .map_err(authority_error)?;
        let git_recovered = self
            .git_issuer
            .recover_expired(OffsetDateTime::now_utc())
            .map_err(runtime_git_authority_error)?;
        usize::try_from(recovered.max(git_recovered))
            .map_err(|_| RunAuthorityError::redacted("recovery count overflowed"))
    }
}

fn authority_error(error: runtime_authority::RuntimeAuthorityError) -> RunAuthorityError {
    RunAuthorityError::redacted(error.to_string())
}

fn runtime_git_authority_error(error: RuntimeGitAuthorityError) -> RunAuthorityError {
    RunAuthorityError::redacted(error.to_string())
}

struct PgAgentVmSpecFactory {
    pool: PgPool,
    root_images: BTreeMap<String, RootFilesystem>,
    runtime_policy: RuntimePolicy,
}

#[derive(Deserialize)]
struct StoredRuntimeContract {
    #[serde(alias = "executable")]
    command: String,
    arguments: Vec<String>,
    working_directory: String,
    image_reference: String,
}

#[derive(Deserialize)]
struct StoredEffectivePolicy {
    vcpus: u8,
    memory_mib: u32,
    network: StoredNetworkAccess,
}

#[derive(Deserialize)]
struct StoredUpdateHook {
    command: String,
    arguments: Vec<String>,
    timeout_seconds: u32,
    resources: StoredHookResources,
}

#[derive(Deserialize)]
struct StoredHookResources {
    vcpus: u8,
    memory_mib: u32,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum StoredNetworkAccess {
    Disabled,
    BrokerOnly,
    Egress,
}

#[async_trait]
impl VmSpecFactory for PgAgentVmSpecFactory {
    // Loading and validating the entire immutable launch contract in one
    // place keeps the no-substitution boundary directly auditable.
    #[allow(clippy::too_many_lines)]
    async fn build(&self, run: &Run) -> Result<VmSpec, VmError> {
        let stored = load_vm_launch_contract(&self.pool, run.id.as_uuid())
            .await
            .map_err(vm_factory_error)?
            .ok_or_else(|| invalid_spec("run", "exact reusable run provenance is missing"))?;
        if stored.release_state != "published" {
            return Err(invalid_spec(
                "release",
                "release is not currently available",
            ));
        }
        if !stored.revision_runnable || !stored.attachment_runnable {
            return Err(invalid_spec(
                "instance_revision",
                "the exact revision or attachment is not runnable",
            ));
        }
        if stored.requires_state != run.requires_state {
            return Err(invalid_spec(
                "requires_state",
                "run state capability does not match the immutable release agent",
            ));
        }
        let contract: StoredRuntimeContract =
            serde_json::from_value(stored.runtime_contract).map_err(vm_factory_error)?;
        let policy: StoredEffectivePolicy =
            serde_json::from_value(stored.effective_runtime_policy).map_err(vm_factory_error)?;
        let root = self
            .root_images
            .get(&contract.image_reference)
            .cloned()
            .ok_or_else(|| invalid_spec("guest.image", "OCI image is not materialized"))?;
        let network_access = policy.network;
        let network = match network_access {
            StoredNetworkAccess::Disabled => NetworkMode::Disabled,
            StoredNetworkAccess::Egress => NetworkMode::UserMode {
                ingress: Vec::new(),
            },
            StoredNetworkAccess::BrokerOnly => NetworkMode::BrokerOnly,
        };
        let (program, arguments, working_directory, resources, timeout_seconds) = match run.kind {
            run_domain::RunKind::Normal => (
                format!("/release/{}", contract.command),
                contract.arguments,
                format!("/release/{}", contract.working_directory),
                VmResources {
                    vcpus: policy.vcpus,
                    memory_mib: policy.memory_mib,
                },
                None,
            ),
            run_domain::RunKind::Update => {
                let hook: StoredUpdateHook = serde_json::from_value(
                    stored
                        .update_hook
                        .ok_or_else(|| invalid_spec("update_hook", "update hook is missing"))?,
                )
                .map_err(vm_factory_error)?;
                (
                    format!("/release/{}", hook.command),
                    hook.arguments,
                    String::from("/release"),
                    VmResources {
                        vcpus: hook.resources.vcpus,
                        memory_mib: hook.resources.memory_mib,
                    },
                    Some(hook.timeout_seconds),
                )
            }
        };
        validate_runtime_policy(&self.runtime_policy, &resources, network_access)?;
        let mut labels = BTreeMap::from([
            (
                String::from("hephaestus.instance"),
                run.instance_id.to_string(),
            ),
            (
                String::from("hephaestus.instance-revision"),
                run.instance_revision_id.to_string(),
            ),
            (
                String::from("hephaestus.release"),
                run.release_id.to_string(),
            ),
            (
                String::from("hephaestus.platform-policy"),
                self.runtime_policy.version.clone(),
            ),
        ]);
        if let Some(seconds) = timeout_seconds {
            labels.insert(
                String::from("hephaestus.wall-clock-timeout-seconds"),
                seconds.to_string(),
            );
        }
        let env = guest_environment(run.kind, stored.agent_update_id)?;
        Ok(VmSpec {
            id: vm_trait::VmId(run.id.to_string()),
            root,
            disks: Vec::new(),
            mounts: Vec::<VmMount>::new(),
            resources,
            network,
            command: GuestCommand {
                program,
                args: arguments,
                env,
                working_dir: Some(working_directory.into()),
            },
            runtime_authority: None,
            private_http_service: None,
            labels,
        })
    }
}

fn guest_environment(
    kind: run_domain::RunKind,
    update_id: Option<Uuid>,
) -> Result<BTreeMap<String, String>, VmError> {
    match (kind, update_id) {
        (run_domain::RunKind::Normal, _) => Ok(BTreeMap::new()),
        (run_domain::RunKind::Update, Some(update_id)) => Ok(BTreeMap::from([(
            String::from("HEPHAESTUS_UPDATE_ID"),
            update_id.to_string(),
        )])),
        (run_domain::RunKind::Update, None) => Err(invalid_spec(
            "update",
            "stable update identity is missing from the hook run",
        )),
    }
}

fn validate_runtime_policy(
    current: &RuntimePolicy,
    resources: &VmResources,
    network: StoredNetworkAccess,
) -> Result<(), VmError> {
    if resources.vcpus > current.max_vcpus || resources.memory_mib > current.max_memory_mib {
        return Err(invalid_spec(
            "effective_runtime_policy.resources",
            "the immutable resource selection exceeds the current platform policy",
        ));
    }
    let network_allowed = match network {
        StoredNetworkAccess::Disabled => true,
        StoredNetworkAccess::BrokerOnly => current.allow_broker_only,
        StoredNetworkAccess::Egress => current.allow_egress,
    };
    if !network_allowed {
        return Err(invalid_spec(
            "effective_runtime_policy.network",
            "the immutable network selection is no longer allowed by platform policy",
        ));
    }
    Ok(())
}

fn invalid_spec(field: &str, reason: &str) -> VmError {
    VmError::InvalidSpec {
        field: field.to_owned(),
        reason: reason.to_owned(),
    }
}

fn vm_factory_error(error: impl std::error::Error + Send + Sync + 'static) -> VmError {
    VmError::Provider {
        provider: String::from("hephaestus-app"),
        code: String::from("spec-factory"),
        source: Box::new(error),
    }
}

struct ResultFixtureProvider;

#[async_trait]
impl VmProvider for ResultFixtureProvider {
    fn name(&self) -> &'static str {
        "local-result-fixture"
    }

    async fn provision(&self, spec: VmSpec) -> Result<Arc<dyn VmInstance>, VmError> {
        Ok(Arc::new(ResultFixtureInstance::new(spec)?))
    }

    async fn cleanup_orphan(&self, _id: &VmId) -> Result<(), VmError> {
        Ok(())
    }
}

struct ResultFixtureInstance {
    id: VmId,
    work: Option<PathBuf>,
    output: Option<PathBuf>,
    exit_code: i32,
    uncertain_exit: bool,
    runtime_authority: Option<(Uuid, u64)>,
    events: broadcast::Sender<VmEvent>,
    exit: watch::Sender<Option<VmExit>>,
}

impl ResultFixtureInstance {
    fn new(spec: VmSpec) -> Result<Self, VmError> {
        let source = spec
            .mounts
            .iter()
            .find(|mount| mount.tag == "repository-source");
        let work = spec
            .mounts
            .iter()
            .find(|mount| mount.tag == "repository-work");
        let work = match (source, work) {
            (Some(source), Some(work)) => {
                if !source.read_only {
                    return Err(invalid_spec(
                        "mounts",
                        "repository source mount must be read-only",
                    ));
                }
                if work.read_only {
                    return Err(invalid_spec(
                        "mounts",
                        "repository work mount must be writable",
                    ));
                }
                Some(work.host_path.clone())
            }
            (None, None) => None,
            _ => {
                return Err(invalid_spec(
                    "mounts",
                    "repository source and work mounts must be paired",
                ));
            }
        };
        let output = spec
            .mounts
            .iter()
            .find(|mount| mount.tag == "build-output")
            .map(|mount| mount.host_path.clone());
        let exit_code = if spec.command.args.iter().any(|value| value == "fail") {
            23
        } else {
            0
        };
        let uncertain_exit = spec.command.args.iter().any(|value| value == "uncertain");
        let runtime_authority = spec
            .runtime_authority
            .as_ref()
            .map(|authority| (authority.session_id(), authority.generation()));
        let (events, _) = broadcast::channel(16);
        let (exit, _) = watch::channel(None);
        Ok(Self {
            id: spec.id,
            work,
            output,
            exit_code,
            uncertain_exit,
            runtime_authority,
            events,
            exit,
        })
    }
}

#[async_trait]
impl VmInstance for ResultFixtureInstance {
    fn id(&self) -> &VmId {
        &self.id
    }

    async fn start(&self) -> Result<(), VmError> {
        drop(self.events.send(VmEvent::Started {
            ingress: Vec::new(),
        }));
        if let Some((session_id, generation)) = self.runtime_authority {
            drop(self.events.send(VmEvent::RuntimeAuthorityAcknowledged {
                session_id,
                generation,
            }));
        }
        drop(self.events.send(VmEvent::Ready));
        if let Some(work) = &self.work {
            tokio::fs::write(
                work.join("input.txt"),
                "agent reviewed and changed this file\n",
            )
            .await
            .map_err(fixture_vm_error)?;
            let reports = work.join("reports");
            tokio::fs::create_dir_all(&reports)
                .await
                .map_err(fixture_vm_error)?;
            tokio::fs::write(reports.join("result.txt"), "durable browser E2E report\n")
                .await
                .map_err(fixture_vm_error)?;
        }
        if let Some(output) = &self.output {
            let reports = output.join("reports");
            tokio::fs::create_dir_all(&reports)
                .await
                .map_err(fixture_vm_error)?;
            tokio::fs::write(reports.join("result.txt"), "built browser artifact\n")
                .await
                .map_err(fixture_vm_error)?;
        }
        drop(self.events.send(VmEvent::Log {
            stream: vm_trait::LogStream::Stdout,
            bytes: b"fixture agent completed workspace edits\n".to_vec(),
        }));
        drop(self.events.send(VmEvent::Metric(VmMetric {
            name: String::from("fixture.cpu_ms"),
            value: 42.0,
            labels: BTreeMap::from([(String::from("phase"), String::from("result"))]),
        })));
        if self.work.is_some() {
            drop(self.events.send(VmEvent::FinalizeResult {
                message: String::from("fixture agent result"),
            }));
        }
        let exit = VmExit {
            code: (!self.uncertain_exit).then_some(self.exit_code),
            signal: self.uncertain_exit.then_some(9),
        };
        drop(self.events.send(VmEvent::Exited(exit.clone())));
        self.exit.send_replace(Some(exit));
        Ok(())
    }

    async fn stop(&self, _mode: StopMode) -> Result<(), VmError> {
        Ok(())
    }

    async fn wait(&self) -> Result<VmExit, VmError> {
        let mut receiver = self.exit.subscribe();
        loop {
            let current_exit = receiver.borrow_and_update().clone();
            if let Some(exit) = current_exit {
                return Ok(exit);
            }
            receiver
                .changed()
                .await
                .map_err(|_| VmError::InvalidState("fixture guest exited without a result"))?;
        }
    }

    fn subscribe_events(&self) -> broadcast::Receiver<VmEvent> {
        self.events.subscribe()
    }

    async fn destroy(&self) -> Result<(), VmError> {
        Ok(())
    }
}

fn fixture_vm_error(error: std::io::Error) -> VmError {
    VmError::Provider {
        provider: String::from("local-result-fixture"),
        code: String::from("workspace-write"),
        source: Box::new(error),
    }
}

async fn verify_database_contract(pool: &PgPool) -> Result<(), AppError> {
    let (migration, melange) = control_plane_postgres::verify_contract(pool)
        .await
        .map_err(component("database contract check"))?;
    if migration != Some(EXPECTED_DATABASE_MIGRATION) {
        return Err(AppError::Configuration(format!(
            "database migration is {migration:?}; expected {EXPECTED_DATABASE_MIGRATION}"
        )));
    }
    if !melange {
        return Err(AppError::Configuration(String::from(
            "Mélange check_permission dispatcher is missing",
        )));
    }
    Ok(())
}

fn component<Error: std::fmt::Display>(
    name: &'static str,
) -> impl FnOnce(Error) -> AppError + Copy {
    move |error| AppError::Component {
        component: name,
        message: error.to_string(),
    }
}

/// Application construction, startup, supervision, or shutdown failure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum AppError {
    /// Static or migration configuration is invalid.
    #[error("invalid application configuration: {0}")]
    Configuration(String),
    /// One application component failed.
    #[error("{component} failed: {message}")]
    Component {
        /// Component name.
        component: &'static str,
        /// Non-sensitive failure.
        message: String,
    },
    /// Readiness barrier failed.
    #[error("application readiness failed: {0}")]
    Readiness(String),
    /// A supervised task failed.
    #[error("supervised application task failed: {0}")]
    Task(String),
    /// An operation timed out.
    #[error("application operation timed out: {0}")]
    Timeout(String),
    /// Resource shutdown failed.
    #[error("application shutdown failed: {0}")]
    Shutdown(String),
}

#[cfg(test)]
mod tests {
    use super::{
        BuildExecutionError, FlushDiagnostics, FlushPublisher, GatewayServiceArtifact,
        GatewayServiceArtifactKind, GatewayServiceIdentity, GatewayServiceMaterializer,
        LocalGatewayReleaseMaterializer, LocalRunRuntimeConfig, LocalRunRuntimeManager,
        MaterializedRoot, OciImageReference, OciWorkerError, RuntimePolicy, StoredNetworkAccess,
        build_delivery_requires_redelivery, deterministic_update_hook_run_id, flush_publisher,
        flush_until_quiescent, guest_environment, refresh_image_filesystem_cache,
        validate_runtime_policy, write_oci_manifest_if_dirty,
    };
    use async_trait::async_trait;
    use gateway_edge::GatewayEdgeError;
    use run_domain::{Run, RunKind};
    use run_orchestrator::{RunRuntimeCatalog, RunRuntimeCatalogError};
    use runtime_types::RunId;
    use sha2::{Digest, Sha256};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::{
        path::PathBuf,
        time::{Duration, Instant},
    };
    use uuid::Uuid;
    use vm_trait::{VmError, VmResources};

    struct EmptyRunRuntimeCatalog;

    #[async_trait]
    impl RunRuntimeCatalog for EmptyRunRuntimeCatalog {
        async fn load_runtime(
            &self,
            _run: &Run,
        ) -> Result<run_orchestrator::RunRuntimeInput, RunRuntimeCatalogError> {
            Err(RunRuntimeCatalogError::Unavailable)
        }

        async fn run_is_live(&self, _run_id: RunId) -> Result<bool, RunRuntimeCatalogError> {
            Ok(false)
        }
    }

    fn policy() -> RuntimePolicy {
        RuntimePolicy {
            version: String::from("test/v2"),
            max_vcpus: 2,
            max_memory_mib: 1_024,
            allow_broker_only: true,
            allow_egress: false,
        }
    }

    #[tokio::test]
    async fn final_outbox_flush_drains_beyond_the_old_pass_cap() {
        let mut passes = 0_u16;
        let deadline = Instant::now() + Duration::from_secs(1);
        let diagnostics = Arc::new(std::sync::Mutex::new(FlushDiagnostics::new(deadline)));
        flush_until_quiescent(deadline, Arc::clone(&diagnostics), || {
            passes += 1;
            let quiescent = passes > 100;
            async move { Ok(quiescent) }
        })
        .await
        .expect("flush reaches quiescence after more than 100 passes");
        assert_eq!(passes, 101);
    }

    #[tokio::test]
    async fn final_outbox_flush_preserves_deadline_failure() {
        let called = Arc::new(AtomicBool::new(false));
        let deadline = Instant::now()
            .checked_sub(Duration::from_secs(1))
            .expect("deadline remains representable");
        let diagnostics = Arc::new(std::sync::Mutex::new(FlushDiagnostics::new(deadline)));
        let result = flush_until_quiescent(deadline, Arc::clone(&diagnostics), {
            let called = Arc::clone(&called);
            move || {
                called.store(true, Ordering::Release);
                async { Ok(false) }
            }
        })
        .await;
        assert!(matches!(
            result,
            Err(super::AppError::Shutdown(message))
                if message == "final outbox flush did not quiesce"
        ));
        assert!(!called.load(Ordering::Acquire));
        let diagnostics = diagnostics.lock().expect("flush diagnostics mutex");
        assert!(diagnostics.deadline_expired_before_pass);
        assert_eq!(diagnostics.passes, 1);
        drop(diagnostics);
    }

    #[tokio::test]
    async fn final_outbox_flush_diagnoses_an_interrupted_publisher() {
        // This crate does not enable Tokio's test clock; leave enough real time for
        // the publisher phase to be entered before the pending operation times out.
        let deadline = Instant::now() + Duration::from_millis(250);
        let diagnostics = Arc::new(std::sync::Mutex::new(FlushDiagnostics::new(deadline)));
        let pass_diagnostics = Arc::clone(&diagnostics);
        let result = flush_until_quiescent(deadline, Arc::clone(&diagnostics), || {
            let diagnostics = Arc::clone(&pass_diagnostics);
            async move {
                let forge = flush_publisher(
                    &diagnostics,
                    FlushPublisher::Forge,
                    async { Ok::<usize, std::io::Error>(3) },
                    "test forge outbox flush",
                )
                .await?;
                assert_eq!(forge, 3);
                let _ = flush_publisher(
                    &diagnostics,
                    FlushPublisher::ProductEvent,
                    std::future::pending::<Result<usize, std::io::Error>>(),
                    "test product-event outbox flush",
                )
                .await?;
                Ok(false)
            }
        })
        .await;
        assert!(matches!(
            result,
            Err(super::AppError::Shutdown(message))
                if message == "final outbox flush did not quiesce"
        ));
        let diagnostics = diagnostics.lock().expect("flush diagnostics mutex");
        assert!(diagnostics.deadline_expired_during_pass);
        assert!(diagnostics.deadline_expired_during_publisher);
        assert_eq!(
            diagnostics.active.map(|phase| phase.publisher.name()),
            Some("product-event")
        );
        assert_eq!(
            diagnostics.last_batches[FlushPublisher::Forge.index()],
            Some(3)
        );
        assert_eq!(
            diagnostics.last_batches[FlushPublisher::ProductEvent.index()],
            None
        );
        assert_eq!(diagnostics.failure_kind, Some("deadline-during-publisher"));
        drop(diagnostics);
    }

    #[tokio::test]
    async fn final_outbox_flush_classifies_publisher_errors() {
        let deadline = Instant::now() + Duration::from_secs(1);
        let diagnostics = Arc::new(std::sync::Mutex::new(FlushDiagnostics::new(deadline)));
        let result = flush_publisher(
            &diagnostics,
            FlushPublisher::ProductEvent,
            async { Err::<usize, _>(std::io::Error::other("publisher unavailable")) },
            "test product-event outbox flush",
        )
        .await;
        assert!(matches!(
            result,
            Err(super::AppError::Component {
                component: "test product-event outbox flush",
                ..
            })
        ));
        let diagnostics = diagnostics.lock().expect("flush diagnostics mutex");
        assert_eq!(diagnostics.failure_kind, Some("publisher-error"));
        assert!(diagnostics.active.is_none());
        assert_eq!(
            diagnostics.last_phase.map(|phase| phase.publisher.name()),
            Some("product-event")
        );
        drop(diagnostics);
    }

    #[test]
    fn app_materializer_bridges_service_identity_and_exact_cleanup() {
        let fixture = tempfile::tempdir().expect("temporary materializer roots");
        let runtime_root = fixture.path().join("runtime");
        let store_root = fixture.path().join("store");
        let key = Uuid::new_v4();
        let bytes = b"persistent service executable";
        std::fs::create_dir(&store_root).expect("store root");
        std::fs::write(store_root.join(key.simple().to_string()), bytes).expect("store object");
        let manager = LocalRunRuntimeManager::initialize(
            Arc::new(EmptyRunRuntimeCatalog),
            LocalRunRuntimeConfig {
                runtime_root: runtime_root.clone(),
                release_artifact_root: store_root,
            },
        )
        .expect("initialize runtime");
        let materializer = LocalGatewayReleaseMaterializer {
            runtime: manager.gateway_release_runtime(),
        };
        let identity = GatewayServiceIdentity {
            instance_id: Uuid::new_v4(),
            gateway_id: Uuid::new_v4(),
            revision_id: Uuid::new_v4(),
        };
        let mounts = materializer
            .prepare_service(
                identity,
                &[GatewayServiceArtifact {
                    path: String::from("bin/server"),
                    kind: GatewayServiceArtifactKind::Executable,
                    mode: 0o555,
                    content_hash: Sha256::digest(bytes).into(),
                    size_bytes: u64::try_from(bytes.len()).expect("artifact length"),
                    storage_key: key,
                }],
                &serde_json::json!({"port": 8080}),
            )
            .expect("materialize service");
        assert_eq!(mounts.len(), 2);
        assert!(mounts.iter().all(|mount| mount.read_only));
        assert_eq!(mounts[0].guest_path, PathBuf::from("/release"));
        assert_eq!(mounts[1].guest_path, PathBuf::from("/run/hephaestus"));

        let wrong_identity = GatewayServiceIdentity {
            gateway_id: Uuid::new_v4(),
            ..identity
        };
        assert!(matches!(
            materializer.destroy_service(wrong_identity),
            Err(GatewayEdgeError::HandlerUnavailable)
        ));
        assert!(runtime_service_path(&runtime_root, identity.instance_id).exists());
        materializer
            .destroy_service(identity)
            .expect("destroy exact service identity");
        assert!(!runtime_service_path(&runtime_root, identity.instance_id).exists());
    }

    fn runtime_service_path(runtime_root: &std::path::Path, instance_id: Uuid) -> PathBuf {
        runtime_root
            .join("gateway-services")
            .join(instance_id.to_string())
    }

    #[test]
    fn recovered_update_hook_attempts_have_distinct_stable_ids() {
        let update_id = Uuid::new_v4();
        let first = deterministic_update_hook_run_id(update_id, 0);
        let retry = deterministic_update_hook_run_id(update_id, 1);
        assert_eq!(first, deterministic_update_hook_run_id(update_id, 0));
        assert_ne!(first, retry);
        assert_eq!(first.as_uuid().get_version_num(), 8);
        assert_eq!(retry.as_uuid().get_version_num(), 8);
    }

    #[test]
    fn current_platform_policy_accepts_an_unchanged_allowed_contract() {
        validate_runtime_policy(
            &policy(),
            &VmResources {
                vcpus: 2,
                memory_mib: 1_024,
            },
            StoredNetworkAccess::BrokerOnly,
        )
        .expect("contract remains allowed");
    }

    #[test]
    fn claimed_build_delivery_is_acknowledged_instead_of_poison_redelivered() {
        assert!(!build_delivery_requires_redelivery(
            &BuildExecutionError::AlreadyClaimed
        ));
        assert!(!build_delivery_requires_redelivery(
            &BuildExecutionError::GuestFailed
        ));
        assert!(build_delivery_requires_redelivery(
            &BuildExecutionError::Release
        ));
        assert!(build_delivery_requires_redelivery(
            &BuildExecutionError::ImageUnavailable
        ));
    }

    #[tokio::test]
    async fn failed_manifest_write_is_retried_when_next_pass_has_no_job() {
        let dirty = AtomicBool::new(false);
        let first = write_oci_manifest_if_dirty(&dirty, true, || async {
            Err(OciWorkerError::ImageNotCached)
        })
        .await;
        assert!(first.is_err());
        assert!(dirty.load(Ordering::Acquire));

        let attempts = AtomicUsize::new(0);
        write_oci_manifest_if_dirty(&dirty, false, || async {
            attempts.fetch_add(1, Ordering::Relaxed);
            Ok(())
        })
        .await
        .expect("dirty manifest is retried without another materialization job");
        assert_eq!(attempts.load(Ordering::Relaxed), 1);
        assert!(!dirty.load(Ordering::Acquire));
    }

    #[test]
    fn durable_materialized_root_hydrates_image_cache() {
        let rootfs = tempfile::tempdir().expect("temporary rootfs");
        let root = rootfs.path().join("materialized");
        std::fs::create_dir(&root).expect("materialized root");
        let reference =
            OciImageReference::parse(format!("localhost/python-ubuntu@sha256:{}", "a".repeat(64)))
                .expect("digest-pinned image reference");
        let image_filesystems =
            std::sync::Arc::new(std::sync::RwLock::new(std::collections::BTreeMap::new()));

        refresh_image_filesystem_cache(
            &[MaterializedRoot {
                image_reference: reference.clone(),
                root_path: root.clone(),
            }],
            rootfs.path(),
            &image_filesystems,
        )
        .expect("durable root refresh");

        let cache = image_filesystems.read().expect("image cache read");
        let Some(vm_trait::RootFilesystem::Directory { host_path }) =
            cache.get(&reference.to_string())
        else {
            panic!("durable root was not hydrated into image cache");
        };
        assert_eq!(
            host_path,
            &std::fs::canonicalize(root).expect("canonical root")
        );
        drop(cache);
    }

    #[test]
    fn current_platform_policy_rejects_stored_resources_over_the_new_ceiling() {
        let error = validate_runtime_policy(
            &policy(),
            &VmResources {
                vcpus: 3,
                memory_mib: 1_024,
            },
            StoredNetworkAccess::Disabled,
        )
        .expect_err("contract exceeds the current ceiling");

        assert!(matches!(
            error,
            VmError::InvalidSpec { ref field, .. }
                if field == "effective_runtime_policy.resources"
        ));
    }

    #[test]
    fn current_platform_policy_rejects_network_access_disabled_since_resolution() {
        let error = validate_runtime_policy(
            &policy(),
            &VmResources {
                vcpus: 1,
                memory_mib: 512,
            },
            StoredNetworkAccess::Egress,
        )
        .expect_err("egress is no longer allowed");

        assert!(matches!(
            error,
            VmError::InvalidSpec { ref field, .. }
                if field == "effective_runtime_policy.network"
        ));
    }

    #[test]
    fn update_guest_receives_the_exact_stable_update_identity() {
        let update_id = Uuid::new_v4();
        let expected = update_id.to_string();
        let environment =
            guest_environment(RunKind::Update, Some(update_id)).expect("update environment");

        assert_eq!(
            environment.get("HEPHAESTUS_UPDATE_ID").map(String::as_str),
            Some(expected.as_str())
        );
        assert!(
            guest_environment(RunKind::Update, None).is_err(),
            "an update hook must fail closed when its durable identity is absent"
        );
        assert!(
            guest_environment(RunKind::Normal, Some(update_id))
                .expect("normal environment")
                .is_empty(),
            "normal agents must not receive an unrelated update identity"
        );
    }
}

#[cfg(test)]
mod gateway_recovery_tests;
