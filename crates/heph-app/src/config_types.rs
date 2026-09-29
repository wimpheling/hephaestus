use super::{
    Algorithm, Arc, BTreeMap, BrokerAdapter, DecodingKey, Duration, EphemeralSecretConfig,
    GitHttpLimits, LibkrunConfig, LocalKeyProvider, LocalOciRuntimeConfig, LocalRunRuntimeConfig,
    LocalVolumeConfig, LocalWorkspaceConfig, OciImageReference, PathBuf, PolicyVersion,
    PublisherConfiguration, RootFilesystem, SocketAddr, SupplyChainPolicy, UiOriginConfig,
    VmProvider, VmResources, ZotClientConfig,
};

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
    /// Optional private UI-origin listener and generation-host policy.
    pub ui_origin: Option<UiOriginConfig>,
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
    /// Explicit provider injection that retains the runtime Git bridge socket.
    ///
    /// Test observers wrap a libkrun provider as a custom provider. Keeping
    /// this metadata beside that provider prevents the composition root from
    /// dropping the bridge endpoint while preserving the observer boundary.
    CustomWithRuntimeGitSocket {
        /// Provider used to provision and run the guest.
        provider: Arc<dyn VmProvider>,
        /// Host Unix socket used by the runtime Git bridge.
        runtime_git_socket_path: PathBuf,
    },
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
