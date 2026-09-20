//! Opt-in daemon-level golden path through every production boundary.

use authz_postgres::PostgresMelangeAuthorizer;
use brokered_egress_domain::{
    BrokeredSecretRule, BrokeredSecretRuleId, ExactHttpsOrigin, HeaderName, HttpInjectionLocation,
};
use forge_domain::{GitRef, OrganizationId, ProjectId};
use forge_postgres::PgForgeRepository;
use forge_service::{CreateRepository, GitStorage};
use hephaestus_app::{
    AppConfig, GatewayEdgeConfig, HephaestusApp, OciBuilderWorkerConfig, OidcConfig,
    RegistryConfig, RunEventKind, UiOriginConfig, VmBackendConfig,
};
use identity_domain::{
    AuthenticatedIdentity, BrowserSessionSid, RequestId, UserId,
    browser_session_identity_binding_digest, browser_session_sid_digest,
};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use mailbox_domain::{
    BodyReference, BodyReferenceId, ContentMetadata, DeduplicationKey, EnvelopeMethod,
    EnvelopeRoute, MailboxEnvelope, MailboxEvent, MailboxId, ProducerId,
};
use mailbox_postgres::PostgresMailboxRepository;
use oci_builder_runtime_local::LocalOciRuntimeConfig;
use registry_domain::{RegistryAuthority, SupplyChainPolicy};
use registry_publisher::PublisherConfiguration;
use registry_token::{RegistryTokenIssuer, SigningKey, TokenLifetime};
use release_service::{
    UiNamespace, UiPublicPort, UiRequestAuditDecision, UiRequestAuditOutcome, UiRequestAuditReason,
    UiRequestAuditSurface,
};
use run_runtime_local::LocalRunRuntimeConfig;
use secret_application::{
    BindSecret, CreateSecret, DeclareBrokeredHttpsRule, GrantAndAcceptSecretImport,
};
use secret_broker::{BrokeredHttpsAdapterRegistry, DenyingBrokerAdapter};
use secret_domain::{
    AgentSecretBindingId, DeliveryMode, ExecutionPhase, SecretAlias, SecretCommandKey,
    SecretGrantId, SecretId, SecretImportId, SecretName, SecretOwner, SecretSlotKey, SecretTarget,
    SecretUsePolicy, SecretValue, SecretVersionId,
};
use secret_postgres::SecretService;
use secret_runtime::EphemeralSecretConfig;
use secret_store::{EncryptedStore, LocalKeyProvider};
use serial_test::serial;
use sha2::{Digest, Sha256};
use sqlx::{Row, postgres::PgPoolOptions};
use std::{
    collections::BTreeMap,
    env,
    fs::{self, OpenOptions},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use time::OffsetDateTime;
use tokio::process::Command;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::Notify,
};
use tokio_rustls::TlsAcceptor;
use url::Url;
use vm_trait::RootFilesystem;
use volume_local::LocalVolumeConfig;
use workspace_local::{LocalWorkspaceConfig, WorkspaceLimits};

#[cfg(feature = "test-fixtures")]
#[path = "composition/gateway_service_log.rs"]
mod gateway_service_log_rpc;
#[cfg(feature = "test-fixtures")]
use gateway_service_log_rpc::{
    GUEST_SERVICE_LOG_STDERR_MARKER, GUEST_SERVICE_LOG_STDOUT_MARKER, GatewayServiceGuestLogProof,
    GatewayServiceLogRpcFixture, marker_count,
};

// Wait for both bounded preparation branches even if an assertion or an
// expected-result check panics. Dropping the sibling could abandon published
// production work; resume the original panic only after both futures settle.
async fn join_cooking_preparation<A, B>(first: A, second: B) -> (A::Output, B::Output)
where
    A: std::future::Future,
    B: std::future::Future,
{
    use futures_util::FutureExt as _;

    let (first, second) = tokio::join!(
        std::panic::AssertUnwindSafe(first).catch_unwind(),
        std::panic::AssertUnwindSafe(second).catch_unwind(),
    );
    (
        first.unwrap_or_else(|panic| std::panic::resume_unwind(panic)),
        second.unwrap_or_else(|panic| std::panic::resume_unwind(panic)),
    )
}

#[tokio::test]
async fn cooking_preparation_overlaps_child_lifecycles() {
    let root = tempfile::tempdir().expect("preparation lifecycle root");
    let child = |own: &'static str, peer: &'static str| {
        let root = root.path();
        async move {
            let status = Command::new("sh")
                .args([
                    "-eu", "-c",
                    r#"touch "$1/$2"; while [ ! -f "$1/$3" ]; do sleep 0.01; done; touch "$1/$2-finished""#,
                    "cooking-preparation", root.to_str().expect("fixture path"), own, peer,
                ])
                .kill_on_drop(true)
                .status()
                .await
                .expect("preparation lifecycle child");
            assert!(status.success());
        }
    };
    // Each real child waits for the other to start. Serial polling deadlocks
    // and hits this bounded fixture timeout instead of passing spuriously.
    tokio::time::timeout(
        Duration::from_secs(5),
        join_cooking_preparation(child("release", "blog"), child("blog", "release")),
    )
    .await
    .expect("both preparation children must make progress");
    assert!(root.path().join("release-finished").is_file());
    assert!(root.path().join("blog-finished").is_file());
}

#[tokio::test]
async fn cooking_preparation_drains_child_before_resuming_either_panic() {
    use futures_util::FutureExt as _;

    for first_panics in [true, false] {
        let root = tempfile::tempdir().expect("preparation failure root");
        let branch = |panics: bool| {
            let root = root.path();
            async move {
                if panics {
                    // Preserve a real unwind payload without printing an expected
                    // fixture panic into the full Cooking workload diagnostics.
                    std::panic::resume_unwind(Box::new("original preparation failure"));
                }
                let status = Command::new("sh")
                    .args([
                        "-eu",
                        "-c",
                        r#"sleep 0.02; touch "$1/finished""#,
                        "cooking-preparation",
                        root.to_str().expect("fixture path"),
                    ])
                    .kill_on_drop(true)
                    .status()
                    .await
                    .expect("preparation cleanup child");
                assert!(status.success());
            }
        };
        let failure = tokio::time::timeout(
            Duration::from_secs(5),
            std::panic::AssertUnwindSafe(join_cooking_preparation(
                branch(first_panics),
                branch(!first_panics),
            ))
            .catch_unwind(),
        )
        .await
        .expect("preparation failure drain deadline")
        .expect_err("original failure must propagate");
        assert_eq!(
            failure.downcast_ref::<&str>(),
            Some(&"original preparation failure"),
        );
        assert!(root.path().join("finished").is_file());
    }
}

const WORKLOAD_PHASE_TIMING_EVENT: &str = "phase-timing";
const WORKLOAD_PHASE_TIMING_MAX_MS: u128 = 45 * 60 * 1_000;

/// Reports bounded timings from the workload process. The trusted supervisor
/// remains authoritative for test outcomes and acceptance decisions.
struct WorkloadPhaseTimer {
    phase: &'static str,
    started: Instant,
    enabled: bool,
    finished: bool,
}

impl WorkloadPhaseTimer {
    fn start(phase: &'static str, enabled: bool) -> Self {
        Self {
            phase,
            started: Instant::now(),
            enabled,
            finished: false,
        }
    }

    fn finish(mut self, success: bool) {
        self.finished = true;
        self.emit(if success { "passed" } else { "failed" });
    }

    fn emit(&self, status: &'static str) {
        if !self.enabled {
            return;
        }
        let elapsed_ms = self.started.elapsed().as_millis();
        let duration_ms = if elapsed_ms <= WORKLOAD_PHASE_TIMING_MAX_MS {
            elapsed_ms
        } else {
            0
        };
        let status = if elapsed_ms <= WORKLOAD_PHASE_TIMING_MAX_MS {
            status
        } else {
            "unknown"
        };
        eprintln!(
            "HEPH_GCP_COOKING event={WORKLOAD_PHASE_TIMING_EVENT} phase={} status={status} duration_ms={duration_ms}",
            self.phase
        );
    }
}

impl Drop for WorkloadPhaseTimer {
    fn drop(&mut self) {
        if !self.finished {
            self.emit("unknown");
        }
    }
}

fn cooking_base_layout(layouts: &BTreeMap<String, PathBuf>, reference: &str) -> Option<PathBuf> {
    let (name, digest) = reference.rsplit_once('@')?;
    let key = name.rsplit('/').next()?;
    let suffix = format!("/{key}@{digest}");
    layouts
        .iter()
        .find(|(candidate, _)| candidate.ends_with(&suffix))
        .map(|(_, path)| path.clone())
}

fn cooking_base_layout_mount_roots(
    layouts: &BTreeMap<String, PathBuf>,
) -> Result<Vec<PathBuf>, (PathBuf, std::io::Error)> {
    let mut roots = layouts
        .values()
        .map(|path| path.canonicalize().map_err(|error| (path.clone(), error)))
        .collect::<Result<Vec<_>, _>>()?;
    roots.sort_unstable();
    roots.dedup();
    Ok(roots)
}

#[test]
fn cooking_base_layout_alias_requires_identical_image_digest() {
    let reviewed = format!(
        "registry.invalid/platform/python-ubuntu@sha256:{}",
        "a".repeat(64)
    );
    let layouts = BTreeMap::from([(reviewed, PathBuf::from("/reviewed/python"))]);
    let same = format!("localhost/python-ubuntu@sha256:{}", "a".repeat(64));
    let different = format!("localhost/python-ubuntu@sha256:{}", "b".repeat(64));
    assert_eq!(
        cooking_base_layout(&layouts, &same),
        Some(PathBuf::from("/reviewed/python"))
    );
    assert_eq!(cooking_base_layout(&layouts, &different), None);
}

#[test]
fn cooking_base_layout_mount_roots_are_canonical_and_exact() {
    let temporary = tempfile::tempdir().expect("golden temporary root");
    let python = temporary.path().join("release/python/image");
    let rust = temporary.path().join("release/rust/image");
    std::fs::create_dir_all(&python).expect("python layout directory");
    std::fs::create_dir_all(&rust).expect("rust layout directory");
    let alias = temporary.path().join("release/python/../python/image");
    let layouts = BTreeMap::from([
        (String::from("python-a"), alias),
        (String::from("python-b"), python.clone()),
        (String::from("rust"), rust.clone()),
    ]);

    assert_eq!(
        cooking_base_layout_mount_roots(&layouts).expect("canonical layout roots"),
        vec![
            python.canonicalize().expect("canonical python layout"),
            rust
        ]
    );
    let missing = BTreeMap::from([(String::from("missing"), temporary.path().join("absent"))]);
    assert!(cooking_base_layout_mount_roots(&missing).is_err());
}

fn workload_phase_timing_from_environment() -> bool {
    env::var_os("HEPH_GCP_PHASE_TIMING_PATH").is_some()
}

fn cooking_oci_worker_config(
    root: &Path,
    repository_root: &Path,
    workload_phase_timing: bool,
) -> Option<OciBuilderWorkerConfig> {
    let builder_image = env::var("HEPHAESTUS_TEST_OCI_BUILDER_VM_IMAGE").ok()?;
    let verifier_image = env::var("HEPHAESTUS_TEST_OCI_VERIFIER_VM_IMAGE").ok()?;
    let base_manifest = PathBuf::from(env::var("HEPHAESTUS_TEST_OCI_BASE_LAYOUT_MANIFEST").ok()?);
    let rootfs_root = PathBuf::from(env::var("HEPHAESTUS_TEST_OCI_ROOTFS_ROOT").ok()?);
    let registry_service = env::var("HEPHAESTUS_TEST_REGISTRY_SERVICE").ok()?;
    let registry_origin = env::var("HEPHAESTUS_TEST_REGISTRY_ORIGIN").ok()?;
    let output_root = root.join("repository-images/candidates");
    let checkout_root = root.join("repository-images/checkouts");
    let verification_root = root.join("repository-images/verification");
    let scratch_root = root.join("repository-images/scratch");
    let credential_root = root.join("repository-images/registry-credentials");
    for path in [
        &output_root,
        &checkout_root,
        &verification_root,
        &scratch_root,
        &credential_root,
    ] {
        std::fs::create_dir_all(path).expect("create cooking OCI worker root");
        std::fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(0o700))
            .expect("set cooking OCI worker root mode");
    }
    let mut image_layouts: std::collections::BTreeMap<String, PathBuf> = serde_json::from_slice(
        &std::fs::read(base_manifest).expect("read cooking OCI base manifest"),
    )
    .expect("parse cooking OCI base manifest");
    // The Hugo image uses the reviewed Python base. Registry names may differ
    // between its local runtime reference and published layout, but the digest
    // must remain identical. A same-name image with different bytes is not an
    // alias and must never stand in for the declared build provenance.
    let python_reference = env::var("HEPHAESTUS_LIBKRUN_UBUNTU_IMAGE")
        .expect("cooking Python image catalog reference");
    let layout = cooking_base_layout(&image_layouts, &python_reference)
        .expect("reviewed Python base layout with the exact cooking image digest");
    image_layouts.insert(python_reference, layout);
    let authority =
        RegistryAuthority::parse(&registry_service).expect("cooking registry authority");
    let skopeo_binary = env::var_os("HEPHAESTUS_SKOPEO")
        .map(PathBuf::from)
        .expect("HEPHAESTUS_SKOPEO must identify the trusted host skopeo binary");
    let oras_binary = env::var_os("HEPHAESTUS_ORAS")
        .map(PathBuf::from)
        .expect("HEPHAESTUS_ORAS must identify the trusted host oras binary");
    let publisher = PublisherConfiguration::new(
        authority,
        &output_root,
        &verification_root,
        &credential_root,
        &skopeo_binary,
        &oras_binary,
    )
    .expect("configure cooking OCI publisher")
    .with_registry_origin(&registry_origin)
    .expect("configure cooking OCI registry origin");
    let guest_init = env::var_os("HEPHAESTUS_GUEST_INIT_BINARY").map_or_else(
        || {
            let target_dir = env::var_os("CARGO_TARGET_DIR")
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target"));
            target_dir.join("x86_64-unknown-linux-musl/release/heph-init")
        },
        PathBuf::from,
    );
    let config = OciBuilderWorkerConfig {
        runtime: LocalOciRuntimeConfig {
            repository_root: repository_root.to_path_buf(),
            checkout_root,
            image_layouts,
            output_root,
            verified_rootfs_root: Some(verification_root.clone()),
            git_binary: PathBuf::from("/usr/bin/git"),
            tar_binary: PathBuf::from("/usr/bin/tar"),
            buildah_binary: None,
            trivy_binary: None,
            umoci_binary: None,
            buildah_output_prefix: String::from("heph-cooking-builder"),
        },
        publisher,
        publication_policy_version: registry_domain::PolicyVersion::parse("cooking/v1")
            .expect("cooking OCI policy version"),
        publication_policy: SupplyChainPolicy::without_signature(),
        builder_vm_image: builder_catalog_domain::OciImageReference::parse(&builder_image)
            .expect("cooking builder image reference"),
        verifier_vm_image: builder_catalog_domain::OciImageReference::parse(&verifier_image)
            .expect("cooking verifier image reference"),
        verification_root,
        scratch_root,
        // Ubuntu packages mkfs.ext4 as a symlink to mke2fs. Resolve the
        // trusted system path before the runtime rejects symlinked executables.
        mkfs_ext4: std::fs::canonicalize("/usr/sbin/mkfs.ext4")
            .expect("canonical mkfs.ext4 must be installed"),
        vm_resources: vm_trait::VmResources {
            vcpus: 2,
            // Buildah plus the libkrun VMM needs the worker's 8 GiB cgroup
            // ceiling for its backing pages; keep the declared guest budget
            // at the reviewed 2 GiB operation allocation.
            memory_mib: 2048,
        },
        workload_phase_timing,
        preparation_worker_name: String::from("golden-cooking-oci-preparation"),
        materialization_worker_name: String::from("golden-cooking-oci-materialization"),
        rootfs_root,
        root_manifest: root.join("repository-builder-roots.json"),
        guest_init,
        lease: Duration::from_secs(900),
        poll_interval: Duration::from_millis(100),
    };
    Some(config)
}

fn golden_registry_config() -> RegistryConfig {
    let local = env::var("HEPHAESTUS_TEST_REGISTRY_SERVICE")
        .ok()
        .zip(env::var("HEPHAESTUS_TEST_REGISTRY_ORIGIN").ok())
        .zip(env::var("HEPHAESTUS_TEST_REGISTRY_PRIVATE_KEY").ok())
        .zip(env::var("HEPHAESTUS_TEST_REGISTRY_KEY_ID").ok());
    let (token_issuer, zot) = if let Some((((service, origin), key_path), key_id)) = local {
        let authority = RegistryAuthority::parse(&service).expect("local golden registry service");
        let key = std::fs::read(key_path).expect("local golden registry signing key");
        let issuer = RegistryTokenIssuer::new(
            "http://127.0.0.1:0/v1/registry/token"
                .parse()
                .expect("local golden registry issuer"),
            service.parse().expect("local golden registry audience"),
            SigningKey::rs256_pem(key_id.parse().expect("local golden registry key ID"), &key)
                .expect("local golden registry signing key material"),
            TokenLifetime::new(300).expect("local golden registry token lifetime"),
        );
        (
            Arc::new(issuer),
            registry_zot::ZotClientConfig::new(authority, &origin)
                .expect("local golden Zot configuration"),
        )
    } else {
        let authority =
            RegistryAuthority::parse("registry.golden.invalid").expect("registry service");
        (
            Arc::new(RegistryTokenIssuer::new(
                "https://forge.golden.invalid/v1/registry/token"
                    .parse()
                    .expect("registry issuer"),
                "registry.golden.invalid".parse().expect("registry service"),
                SigningKey::hs256(
                    "golden-v1".parse().expect("registry key id"),
                    SIGNING_SECRET,
                )
                .expect("registry signing key"),
                TokenLifetime::new(300).expect("registry token lifetime"),
            )),
            registry_zot::ZotClientConfig::new(authority, "http://127.0.0.1:1/")
                .expect("Zot client configuration"),
        )
    };
    RegistryConfig {
        token_issuer,
        notification_callback: registry_notification::CallbackCredential::parse(
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        )
        .expect("registry notification callback"),
        zot,
        reconciliation_lease: Duration::from_secs(30),
        reconciliation_interval: Duration::from_secs(30),
    }
}

#[path = "../../../examples/cooking/tests/scenario.rs"]
mod cooking;
#[path = "../../../examples/cooking/tests/adversarial_agent.rs"]
mod cooking_adversarial_agent;
#[path = "../../../examples/cooking/tests/authority.rs"]
mod cooking_authority;
#[path = "../../../examples/cooking/tests/blog_artifact.rs"]
mod cooking_blog_artifact;
#[path = "../../../examples/cooking/tests/builds.rs"]
mod cooking_builds;
#[path = "../../../examples/cooking/tests/confinement.rs"]
mod cooking_confinement;
#[path = "../../../examples/cooking/tests/conflicts.rs"]
mod cooking_conflicts;
#[path = "../../../examples/cooking/tests/guest_crash.rs"]
mod cooking_guest_crash;
#[path = "../../../examples/cooking/tests/ingress_loss.rs"]
mod cooking_ingress_loss;
#[path = "../../../examples/cooking/tests/inspection.rs"]
mod cooking_inspection;
#[path = "../../../examples/cooking/tests/retirement.rs"]
mod cooking_retirement;
#[path = "../../../examples/cooking/tests/service_build.rs"]
mod cooking_service_build;
#[path = "../../../examples/cooking/tests/updates.rs"]
mod cooking_updates;
#[path = "service_helpers/revocation.rs"]
mod service_revocation;
// The integration-test support tree is private to this test crate; its
// `pub(crate)` child boundaries are required by sibling fixture modules.
#[allow(clippy::redundant_pub_crate)]
mod support;

use support::backend_fixture;

const ISSUER: &str = "https://issuer.golden.invalid";

/// Returns the issuer used by this golden run, including the isolated issuer
/// supplied when the cooking browser attaches to the live daemon.
pub(crate) fn golden_issuer() -> String {
    env::var("HEPHAESTUS_COOKING_BROWSER_OIDC_ISSUER").unwrap_or_else(|_| ISSUER.to_owned())
}
const AUDIENCE: &str = "hephaestus-git";
const SIGNING_SECRET: &[u8] = b"golden-test-signing-secret-with-sufficient-entropy";
const ROOT_IMAGE: &str =
    "golden-root@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const BROKERED_E2E_RULE_ID: uuid::Uuid = uuid::uuid!("00000000-0000-0000-0000-000000000002");
const BROKERED_E2E_SENTINEL: &str = "golden-brokered-provider-sentinel-5d1a";
const GATEWAY_HANDLER: &str =
    "#!/bin/sh\nexec /usr/libexec/hephaestus/integration-check --private-http-brokered-mailbox\n";
const SERVICE_GATEWAY_HANDLER: &str =
    "#!/bin/sh\nexec /usr/libexec/hephaestus/integration-check --serve-service\n";

async fn restart_application(config: AppConfig) -> hephaestus_app::RunningHephaestus {
    HephaestusApp::build(config)
        .await
        .expect("rebuild production application after restart")
        .start()
        .await
        .expect("restart ready application")
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ProductOutboxCensus {
    total: i64,
    pending: i64,
    published: i64,
    dead_lettered: i64,
    pending_fingerprint: String,
}

struct IsolatedGoldenDatabase {
    database_name: String,
    maintenance_url: String,
    target_url: String,
}

/// Verifies only the redacted, safe context emitted by the installed UI
/// browser flow. The request audit stream deliberately has no route, query,
/// cookie, credential, or response-payload columns for this observer to read.
async fn assert_installed_ui_audit_success(
    pool: &sqlx::PgPool,
    actor_id: uuid::Uuid,
    organization_id: uuid::Uuid,
    installation_id: uuid::Uuid,
    generation_id: uuid::Uuid,
    surface: UiRequestAuditSurface,
    child_required: bool,
) -> Vec<uuid::Uuid> {
    type AuditRow = (
        uuid::Uuid,
        Option<uuid::Uuid>,
        Option<uuid::Uuid>,
        Option<uuid::Uuid>,
        Option<uuid::Uuid>,
        Option<uuid::Uuid>,
        Option<uuid::Uuid>,
        Option<uuid::Uuid>,
    );
    let rows: Vec<AuditRow> = sqlx::query_as(
        "SELECT request_id, actor_id, organization_id, installation_id,
                generation_id, child_session_id, gateway_id, gateway_revision_id
           FROM ui_request_audit_events
          WHERE installation_id = $1
            AND generation_id = $2
            AND surface = $3
            AND decision = $4
            AND outcome = $5
            AND reason_code = $6
          ORDER BY occurred_at, id",
    )
    .bind(installation_id)
    .bind(generation_id)
    .bind(surface.as_str())
    .bind(UiRequestAuditDecision::Allowed.as_str())
    .bind(UiRequestAuditOutcome::Succeeded.as_str())
    .bind(UiRequestAuditReason::None.as_str())
    .fetch_all(pool)
    .await
    .expect("read installed UI request audit rows");
    assert!(
        !rows.is_empty(),
        "installed UI must emit a successful {} audit row",
        surface.as_str()
    );
    for row in &rows {
        assert_eq!(row.1, Some(actor_id), "{} audit actor", surface.as_str());
        assert_eq!(
            row.2,
            Some(organization_id),
            "{} audit organization",
            surface.as_str()
        );
        assert_eq!(
            row.3,
            Some(installation_id),
            "{} audit installation",
            surface.as_str()
        );
        assert_eq!(
            row.4,
            Some(generation_id),
            "{} audit generation",
            surface.as_str()
        );
        assert_eq!(
            row.5.is_some(),
            child_required,
            "{} audit child-session context",
            surface.as_str()
        );
        assert_eq!(
            row.6.is_some(),
            row.7.is_some(),
            "{} audit gateway context must be paired",
            surface.as_str()
        );
    }
    rows.into_iter().map(|row| row.0).collect()
}

struct InstalledUiBrowserContext<'a> {
    pool: &'a sqlx::PgPool,
    running: &'a hephaestus_app::RunningHephaestus,
    database_url: &'a str,
    organization_id: OrganizationId,
    installed_uis: cooking_builds::InstalledCookingReferenceUis,
    actor_id: uuid::Uuid,
    workload_phase_timing: bool,
}

/// Runs the installed-reference-UI browser phase while the service-proof
/// daemon, Caddy, database, and managed gateway are still alive.  The
/// service-proof branch otherwise tears those resources down before reaching
/// the ordinary browser block below.
async fn run_installed_ui_browser_phase(context: InstalledUiBrowserContext<'_>) {
    let fixture_path = env::var("HEPHAESTUS_COOKING_BROWSER_FIXTURE_OUTPUT")
        .expect("installed UI browser fixture output path");
    let fixture = serde_json::json!({
        "installed_reference_uis": {
            "project_id": context.installed_uis.project_id,
            "static_installation_id": context.installed_uis.static_ui.installation_id,
            "managed_installation_id": context.installed_uis.managed_ui.installation_id
        }
    });
    tokio::fs::write(
        &fixture_path,
        serde_json::to_vec_pretty(&fixture).expect("installed UI browser fixture JSON"),
    )
    .await
    .expect("write installed UI browser fixture JSON");
    let issuer = env::var("HEPHAESTUS_COOKING_BROWSER_OIDC_ISSUER")
        .expect("installed UI browser OIDC issuer");
    let script =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scripts/run-installed-ui-e2e.sh");
    let browser_timer = WorkloadPhaseTimer::start("browser-initial", context.workload_phase_timing);
    let status = tokio::process::Command::new(script)
        .env("HEPHAESTUS_E2E_COOKING_FIXTURE", &fixture_path)
        .env("HEPHAESTUS_E2E_EXTERNAL_DATABASE_URL", context.database_url)
        .env(
            "HEPHAESTUS_E2E_EXTERNAL_RPC_ENDPOINT",
            context.running.http_addr().to_string(),
        )
        .env(
            "HEPHAESTUS_E2E_EXTERNAL_RPC_SECRET",
            "golden-internal-command-token-with-sufficient-entropy",
        )
        .env("HEPHAESTUS_E2E_EXTERNAL_OIDC_ISSUER", issuer)
        .env("HEPHAESTUS_E2E_COOKING_PHASE", "initial")
        .env(
            "HEPHAESTUS_PLATFORM_HTTPS_ORIGIN",
            installed_ui_platform_origin(),
        )
        .env("HEPHAESTUS_UI_NAMESPACE", installed_ui_namespace())
        .env(
            "HEPHAESTUS_CADDY_TEST_CA_CERT",
            env::var("HEPHAESTUS_CADDY_TEST_CA_CERT")
                .expect("joined Caddy CA certificate for installed UI"),
        )
        .status()
        .await;
    browser_timer.finish(status.as_ref().is_ok_and(std::process::ExitStatus::success));
    let status = status.expect("run installed UI browser E2E");
    assert!(
        status.success(),
        "installed UI browser E2E failed: {status}"
    );

    assert_installed_ui_browser_audit(&context).await;
}

async fn assert_installed_ui_browser_audit(context: &InstalledUiBrowserContext<'_>) {
    let static_installation_id = context.installed_uis.static_ui.installation_id;
    let static_generation_id = context.installed_uis.static_ui.generation_id;
    for surface in [
        UiRequestAuditSurface::HandoffIssue,
        UiRequestAuditSurface::HandoffExchange,
        UiRequestAuditSurface::Bootstrap,
        UiRequestAuditSurface::Static,
    ] {
        assert_installed_ui_audit_success(
            context.pool,
            context.actor_id,
            context.organization_id.as_uuid(),
            static_installation_id,
            static_generation_id,
            surface,
            surface != UiRequestAuditSurface::HandoffIssue,
        )
        .await;
    }
    let managed_installation_id = context.installed_uis.managed_ui.installation_id;
    let managed_generation_id = context.installed_uis.managed_ui.generation_id;
    for surface in [
        UiRequestAuditSurface::HandoffIssue,
        UiRequestAuditSurface::HandoffExchange,
        UiRequestAuditSurface::Bootstrap,
        UiRequestAuditSurface::Managed,
        UiRequestAuditSurface::Embed,
    ] {
        assert_installed_ui_audit_success(
            context.pool,
            context.actor_id,
            context.organization_id.as_uuid(),
            managed_installation_id,
            managed_generation_id,
            surface,
            !matches!(
                surface,
                UiRequestAuditSurface::HandoffIssue | UiRequestAuditSurface::Embed
            ),
        )
        .await;
    }
    let api_request_ids = assert_installed_ui_audit_success(
        context.pool,
        context.actor_id,
        context.organization_id.as_uuid(),
        managed_installation_id,
        managed_generation_id,
        UiRequestAuditSurface::Api,
        true,
    )
    .await;
    let correlated_api_count: i64 = sqlx::query_scalar(
        "SELECT count(*)
           FROM gateway_invocations invocation
           JOIN ui_request_audit_events audit
             ON audit.request_id = invocation.request_id
          WHERE audit.installation_id = $1
            AND audit.generation_id = $2
            AND audit.surface = $3
            AND audit.request_id = ANY($4)",
    )
    .bind(managed_installation_id)
    .bind(managed_generation_id)
    .bind(UiRequestAuditSurface::Api.as_str())
    .bind(&api_request_ids)
    .fetch_one(context.pool)
    .await
    .expect("read installed UI gateway invocation correlation");
    assert!(
        correlated_api_count > 0,
        "managed API audit must correlate to a gateway invocation"
    );
    println!("REAL_UI_INSTALLATION_AUDIT=1 static=1 managed=1 api=1 embed=1 gateway_correlation=1");
}

impl IsolatedGoldenDatabase {
    async fn create(parent_url: &str) -> Self {
        let database_name = format!("hephaestus_golden_{}", uuid::Uuid::new_v4().simple());
        let mut maintenance_url = Url::parse(parent_url).expect("parse golden PostgreSQL URL");
        maintenance_url.set_path("/postgres");
        let mut target_url = Url::parse(parent_url).expect("parse golden PostgreSQL URL");
        target_url.set_path(&format!("/{database_name}"));
        let maintenance_pool = PgPoolOptions::new()
            .max_connections(2)
            .connect(maintenance_url.as_str())
            .await
            .expect("connect golden PostgreSQL maintenance database");
        let create_sql = format!("CREATE DATABASE \"{database_name}\"");
        sqlx::query(&create_sql)
            .execute(&maintenance_pool)
            .await
            .expect("create isolated golden database");
        maintenance_pool.close().await;

        let target_pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(target_url.as_str())
            .await
            .expect("connect isolated golden database");
        sqlx::migrate!("../../migrations")
            .run(&target_pool)
            .await
            .expect("apply migrations to isolated golden database");
        target_pool.close().await;

        Self {
            database_name,
            maintenance_url: maintenance_url.to_string(),
            target_url: target_url.to_string(),
        }
    }

    async fn cleanup(self, pool: sqlx::PgPool) {
        pool.close().await;
        let maintenance_pool = PgPoolOptions::new()
            .max_connections(1)
            .connect(&self.maintenance_url)
            .await
            .expect("reconnect golden PostgreSQL maintenance database");
        let drop_sql = format!("DROP DATABASE \"{}\"", self.database_name);
        sqlx::query(&drop_sql)
            .execute(&maintenance_pool)
            .await
            .expect("drop isolated golden database after all pools closed");
        maintenance_pool.close().await;
    }
}

async fn product_outbox_census(database_url: &str) -> ProductOutboxCensus {
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(database_url)
        .await
        .expect("connect product outbox census database");
    let row = sqlx::query(
        "SELECT count(*)::bigint AS total,
                count(*) FILTER (
                    WHERE published_at IS NULL AND dead_lettered_at IS NULL
                )::bigint AS pending,
                count(*) FILTER (WHERE published_at IS NOT NULL)::bigint AS published,
                count(*) FILTER (WHERE dead_lettered_at IS NOT NULL)::bigint AS dead_lettered,
                md5(COALESCE(string_agg(
                    event_id::text, ',' ORDER BY event_id
                ) FILTER (
                    WHERE published_at IS NULL AND dead_lettered_at IS NULL
                ), '')) AS pending_fingerprint
           FROM product_event_outbox",
    )
    .fetch_one(&pool)
    .await
    .expect("read product outbox census");
    let census = ProductOutboxCensus {
        total: row.get("total"),
        pending: row.get("pending"),
        published: row.get("published"),
        dead_lettered: row.get("dead_lettered"),
        pending_fingerprint: row.get("pending_fingerprint"),
    };
    pool.close().await;
    census
}

async fn finish_isolated_golden(
    isolated: IsolatedGoldenDatabase,
    pool: sqlx::PgPool,
    target_url: &str,
    parent_url: &str,
    parent_before: Option<ProductOutboxCensus>,
) {
    let target_after = product_outbox_census(target_url).await;
    let parent_after = parent_before
        .as_ref()
        .map(|_| async { product_outbox_census(parent_url).await });
    isolated.cleanup(pool).await;
    assert_eq!(
        target_after.pending, 0,
        "isolated golden outbox must be quiescent before database cleanup"
    );
    if let Some(parent_before) = parent_before {
        let parent_after = parent_after.expect("parent census future").await;
        assert_eq!(
            parent_after, parent_before,
            "golden bootstrap must not mutate the inherited parent outbox"
        );
    }
}

/// Runs the first persistent-service proof in a real external daemon process.
///
/// Keeping this opt-in path beside the existing golden setup is deliberate:
/// the test still owns the exact release/declaration fixture, while the child
/// consumes only the production environment contract used by `hephaestusd`.
/// Later tests can replace the graceful signal below with an unclean process
/// termination without changing the fixture or Caddy wiring.
// Keep the external daemon fixture arguments explicit so each owned resource
// and its cleanup boundary remain visible at the opt-in proof entry point.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
async fn exercise_external_gateway_service_warm_path(
    pool: &sqlx::PgPool,
    fixture: &GatewayServiceGoldenFixture,
    gateway: &GatewayEdgeConfig,
    app_config: &AppConfig,
    root: &Path,
    root_image: &Path,
    release_artifact_root: &Path,
    owner: &AuthenticatedIdentity,
) {
    let mut daemon =
        spawn_external_golden_daemon(gateway, app_config, root, root_image, None).await;
    wait_for_external_daemon_health(&mut daemon).await;
    let first_instance_id = wait_for_gateway_service_ready(pool, fixture).await;
    let paths = gateway_service_resource_paths(first_instance_id);
    assert!(paths.0.is_dir(), "external service VM runtime exists");
    assert!(paths.1.is_dir(), "external service cgroup exists");
    assert!(paths.2.is_dir(), "external service materializer exists");

    let applied_config =
        wait_for_caddy_configuration(&gateway.caddy_admin_url, "/gateway/service").await;
    assert!(
        applied_config.contains("/gateway/service"),
        "external daemon must publish the persistent service Caddy route"
    );
    let public_url = env::var("HEPHAESTUS_CADDY_TEST_PUBLIC_URL")
        .expect("joined Caddy public URL for external daemon proof");
    let first_proof = exercise_gateway_service_requests(&public_url).await;
    let cutover = env::var("HEPHAESTUS_APP_GATEWAY_SERVICE_CUTOVER_E2E").as_deref() == Ok("1");
    let rollback = env::var("HEPHAESTUS_APP_GATEWAY_SERVICE_ROLLBACK_E2E").as_deref() == Ok("1");
    let unclean_restart =
        env::var("HEPHAESTUS_APP_GATEWAY_SERVICE_EXTERNAL_CRASH_E2E").as_deref() == Ok("1");
    let failed_candidate =
        env::var("HEPHAESTUS_APP_GATEWAY_SERVICE_FAILED_CANDIDATE_E2E").as_deref() == Ok("1");
    let candidate_capacity =
        env::var("HEPHAESTUS_APP_GATEWAY_SERVICE_CANDIDATE_CAPACITY_E2E").as_deref() == Ok("1");
    let revocation =
        env::var("HEPHAESTUS_APP_GATEWAY_SERVICE_REVOCATION_E2E").as_deref() == Ok("1");
    if revocation {
        service_revocation::exercise_external_gateway_service_revocation(
            pool,
            fixture,
            &daemon,
            first_instance_id,
            &paths,
            &public_url,
            owner,
        )
        .await;
        daemon.graceful_shutdown().await;
        eprintln!(
            "REAL_GATEWAY_SERVICE_REVOCATION_E2E=1 gateway_id={} revision_id={} instance_id={}",
            fixture.gateway_id, fixture.revision_id, first_instance_id
        );
    } else if failed_candidate {
        exercise_external_gateway_service_failed_candidate(
            pool,
            fixture,
            daemon,
            first_instance_id,
            paths,
            &first_proof,
            &public_url,
            release_artifact_root,
        )
        .await;
    } else if cutover {
        exercise_external_gateway_service_cutover(
            pool,
            fixture,
            gateway,
            app_config,
            root,
            root_image,
            release_artifact_root,
            daemon,
            first_instance_id,
            paths,
            &first_proof,
            &public_url,
            rollback,
            candidate_capacity,
        )
        .await;
    } else if unclean_restart {
        exercise_external_gateway_service_unclean_restart(ExternalGatewayServiceUncleanRestart {
            pool,
            fixture,
            gateway,
            app_config,
            root,
            root_image,
            daemon,
            first_instance_id,
            old_paths: paths,
            first_proof: &first_proof,
            public_url: &public_url,
        })
        .await;
    } else {
        daemon.graceful_shutdown().await;
        wait_for_gateway_service_cleaned(pool, fixture, first_instance_id).await;
        assert!(!paths.0.exists(), "external service VM runtime is cleaned");
        assert!(!paths.1.exists(), "external service cgroup is cleaned");
        assert!(
            !paths.2.exists(),
            "external service materializer is cleaned"
        );
    }
    assert!(!first_proof.startup_id.is_empty());
    eprintln!(
        "persistent-service-external-warm-evidence instance={first_instance_id} startup_id={}",
        first_proof.startup_id
    );
    eprintln!("persistent-service-external-warm-passed");
}

struct ExternalGatewayServiceUncleanRestart<'a> {
    pool: &'a sqlx::PgPool,
    fixture: &'a GatewayServiceGoldenFixture,
    gateway: &'a GatewayEdgeConfig,
    app_config: &'a AppConfig,
    root: &'a Path,
    root_image: &'a Path,
    daemon: ExternalGoldenDaemon,
    first_instance_id: uuid::Uuid,
    old_paths: (PathBuf, PathBuf, PathBuf),
    first_proof: &'a GatewayServiceRequestProof,
    public_url: &'a str,
}

async fn exercise_external_gateway_service_unclean_restart(
    request: ExternalGatewayServiceUncleanRestart<'_>,
) {
    let ExternalGatewayServiceUncleanRestart {
        pool,
        fixture,
        gateway,
        app_config,
        root,
        root_image,
        daemon,
        first_instance_id,
        old_paths,
        first_proof,
        public_url,
    } = request;
    let old_ownership = read_gateway_service_ownership(pool, first_instance_id).await;
    let db_now: OffsetDateTime = sqlx::query_scalar("SELECT clock_timestamp()")
        .fetch_one(pool)
        .await
        .expect("read database clock before daemon crash");
    assert!(
        old_ownership.3 > db_now,
        "service lease must be live before the daemon crash"
    );
    let exit_status = daemon.unclean_kill().await;
    assert_eq!(
        std::os::unix::process::ExitStatusExt::signal(&exit_status),
        Some(9),
        "external daemon must be terminated by SIGKILL"
    );
    eprintln!(
        "persistent-service-unclean-daemon-kill old_instance={first_instance_id} old_fence={} old_lease_expires_at={}",
        old_ownership.2, old_ownership.3
    );
    let old_resources_after_kill = (
        old_paths.0.exists(),
        old_paths.1.exists(),
        old_paths.2.exists(),
    );
    eprintln!(
        "persistent-service-unclean-daemon-residual-resources runtime={} cgroup={} materializer={}",
        old_resources_after_kill.0, old_resources_after_kill.1, old_resources_after_kill.2
    );
    let recovery_started_at: OffsetDateTime = sqlx::query_scalar("SELECT clock_timestamp()")
        .fetch_one(pool)
        .await
        .expect("read database clock before daemon restart");
    let restart_log = root.join("external-hephaestusd-restart.log");
    let mut restarted =
        spawn_external_golden_daemon(gateway, app_config, root, root_image, Some(&restart_log))
            .await;
    wait_for_external_daemon_health(&mut restarted).await;
    let replacement_instance_id = wait_for_gateway_service_boot_replacement(
        pool,
        fixture,
        first_instance_id,
        old_ownership,
        old_paths,
        recovery_started_at,
    )
    .await;
    let replacement_paths = gateway_service_resource_paths(replacement_instance_id);
    let replacement_proof = exercise_gateway_service_requests(public_url).await;
    assert_ne!(
        replacement_instance_id, first_instance_id,
        "unclean daemon restart must claim a fresh service instance"
    );
    assert_ne!(
        replacement_proof.startup_id, first_proof.startup_id,
        "unclean daemon restart must start a fresh guest process"
    );
    assert!(
        replacement_paths.0.is_dir(),
        "replacement VM runtime must exist before final shutdown"
    );
    assert!(
        replacement_paths.1.is_dir(),
        "replacement cgroup must exist before final shutdown"
    );
    assert!(
        replacement_paths.2.is_dir(),
        "replacement materializer must exist before final shutdown"
    );
    eprintln!(
        "persistent-service-unclean-daemon-recovered old_instance={first_instance_id} replacement_instance={replacement_instance_id} replacement_startup_id={}",
        replacement_proof.startup_id
    );
    restarted.graceful_shutdown().await;
    wait_for_gateway_service_cleaned(pool, fixture, replacement_instance_id).await;
    assert!(!replacement_paths.0.exists());
    assert!(!replacement_paths.1.exists());
    assert!(!replacement_paths.2.exists());
    eprintln!("persistent-service-unclean-daemon-recovery-passed");
}

/// Exercises a real failed candidate while the original ready revision keeps
/// serving public identity requests. The `/crash` readiness probe returns the
/// fixture's 503 before its process exits with 42; this acceptance requires
/// the durable unexpected-exit report so a generic startup failure cannot be
/// mistaken for guest execution.
#[allow(
    // This one acceptance path keeps failure, public continuity, and cleanup
    // assertions together so their ordering remains explicit.
    clippy::cognitive_complexity,
    clippy::too_many_arguments,
    clippy::too_many_lines
)]
async fn exercise_external_gateway_service_failed_candidate(
    pool: &sqlx::PgPool,
    fixture: &GatewayServiceGoldenFixture,
    daemon: ExternalGoldenDaemon,
    old_instance_id: uuid::Uuid,
    old_paths: (PathBuf, PathBuf, PathBuf),
    first_proof: &GatewayServiceRequestProof,
    public_url: &str,
    release_artifact_root: &Path,
) {
    let old_fencing_token: i64 = sqlx::query_scalar(
        "SELECT fencing_token
           FROM gateway_service_instances
          WHERE id = $1 AND gateway_id = $2 AND revision_id = $3",
    )
    .bind(old_instance_id)
    .bind(fixture.gateway_id)
    .bind(fixture.revision_id)
    .fetch_one(pool)
    .await
    .expect("read A fencing token before failed candidate");
    assert!(old_paths.0.is_dir() && old_paths.1.is_dir() && old_paths.2.is_dir());

    let candidate =
        seed_gateway_service_cutover_candidate(pool, fixture, release_artifact_root, "/crash")
            .await;
    let gateway_events_before_declaration: i64 = sqlx::query_scalar(
        "SELECT count(*)
           FROM application_events
          WHERE aggregate_type = 'gateway' AND aggregate_id = $1",
    )
    .bind(fixture.gateway_id)
    .fetch_one(pool)
    .await
    .expect("count gateway events before failed candidate declaration");
    sqlx::query("UPDATE gateways SET desired_service_revision_id = $2 WHERE id = $1")
        .bind(fixture.gateway_id)
        .bind(candidate.revision_id)
        .execute(pool)
        .await
        .expect("declare failed published candidate");
    let gateway_events_after_declaration: i64 = sqlx::query_scalar(
        "SELECT count(*)
           FROM application_events
          WHERE aggregate_type = 'gateway' AND aggregate_id = $1",
    )
    .bind(fixture.gateway_id)
    .fetch_one(pool)
    .await
    .expect("count gateway events after failed candidate declaration");
    assert_eq!(
        gateway_events_after_declaration,
        gateway_events_before_declaration + 1,
        "declaring failed B emits exactly one durable gateway event"
    );

    // A bounded request after declaration confirms that the existing public
    // revision remains the one served while B is being attempted.
    let during_startup = exercise_gateway_service_requests(public_url).await;
    assert_eq!(during_startup.startup_id, first_proof.startup_id);
    let evidence = wait_for_failed_gateway_service_candidate(
        pool,
        fixture.gateway_id,
        candidate.revision_id,
        (old_instance_id, fixture.revision_id, old_fencing_token),
        first_proof,
        public_url,
    )
    .await;

    assert_eq!(evidence.gateway_id, fixture.gateway_id);
    assert_eq!(evidence.revision_id, candidate.revision_id);
    assert!(
        !evidence.observed_ready,
        "failed B must not be observed Ready during lifecycle polling"
    );
    assert!(
        !evidence.observed_promoted,
        "failed B must not be observed as the active revision during polling"
    );
    assert_eq!(evidence.state, "cleaned");
    let failure_code = evidence
        .failure_code
        .as_deref()
        .expect("failed B retains a durable failure classification");
    assert_eq!(
        failure_code, "unexpected_exit",
        "failed-candidate proof must observe the guest crash, not startup failure"
    );
    assert_eq!(evidence.exit_code, Some(42));
    assert!(evidence.exit_signal.is_none());
    assert!(evidence.failed_at.is_some());
    assert_eq!(evidence.fencing_token, evidence.initial_fencing_token);

    let b_invocations: i64 = sqlx::query_scalar(
        "SELECT count(*)
           FROM gateway_invocations
          WHERE gateway_id = $1
            AND gateway_revision_id = $2",
    )
    .bind(fixture.gateway_id)
    .bind(candidate.revision_id)
    .fetch_one(pool)
    .await
    .expect("count all failed B invocations");
    assert_eq!(
        b_invocations, 0,
        "failed B must not receive invocations of any outcome"
    );
    let b_bound_invocations: i64 = sqlx::query_scalar(
        "SELECT count(*)
           FROM gateway_invocations
          WHERE gateway_id = $1
            AND gateway_revision_id = $2
            AND service_instance_id = $3
            AND service_instance_fencing_token = $4",
    )
    .bind(fixture.gateway_id)
    .bind(candidate.revision_id)
    .bind(evidence.instance_id)
    .bind(evidence.fencing_token)
    .fetch_one(pool)
    .await
    .expect("count failed B invocations bound to its exact lease");
    assert_eq!(
        b_bound_invocations, 0,
        "failed B must not receive invocations on its exact instance and fence"
    );

    let gateway_events_after_cleanup: i64 = sqlx::query_scalar(
        "SELECT count(*)
           FROM application_events
          WHERE aggregate_type = 'gateway' AND aggregate_id = $1",
    )
    .bind(fixture.gateway_id)
    .fetch_one(pool)
    .await
    .expect("count gateway events after failed candidate cleanup");
    assert_eq!(
        gateway_events_after_cleanup, gateway_events_after_declaration,
        "failed B cleanup must not promote or roll back a gateway pointer"
    );

    let (active_revision, old_state): (Option<uuid::Uuid>, String) = sqlx::query_as(
        "SELECT gateway.active_revision_id, instance.state
           FROM gateways AS gateway
           JOIN gateway_service_instances AS instance
             ON instance.id = $2
            AND instance.gateway_id = gateway.id
            AND instance.revision_id = $3
          WHERE gateway.id = $1",
    )
    .bind(fixture.gateway_id)
    .bind(old_instance_id)
    .bind(fixture.revision_id)
    .fetch_one(pool)
    .await
    .expect("read active A after failed B cleanup");
    assert_eq!(active_revision, Some(fixture.revision_id));
    assert_eq!(old_state, "ready");

    let retry = evidence
        .retry
        .expect("failed B records a revision-scoped retry snapshot");
    assert!(retry.0 >= 1, "failed B increments durable retry streak");
    let failed_at = evidence
        .failed_at
        .expect("failed B includes durable failure timestamp");
    let minimum_retry_delay = match retry.0 {
        1 => 1,
        2 => 2,
        3 => 4,
        4 => 8,
        5 => 16,
        6 => 32,
        _ => 60,
    };
    let next_retry_at = retry
        .1
        .expect("failed B retry snapshot includes next retry timestamp");
    assert!(
        next_retry_at >= failed_at + time::Duration::seconds(minimum_retry_delay),
        "failed B retry must honor durable failure backoff"
    );

    let after_cleanup = exercise_gateway_service_requests(public_url).await;
    assert_eq!(after_cleanup.startup_id, first_proof.startup_id);
    let old_fence_after: i64 = sqlx::query_scalar(
        "SELECT fencing_token
           FROM gateway_service_instances
          WHERE id = $1 AND gateway_id = $2 AND revision_id = $3",
    )
    .bind(old_instance_id)
    .bind(fixture.gateway_id)
    .bind(fixture.revision_id)
    .fetch_one(pool)
    .await
    .expect("read A fencing token after failed B cleanup");
    assert_eq!(old_fence_after, old_fencing_token);

    let b_paths = gateway_service_resource_paths(evidence.instance_id);
    assert!(!b_paths.0.exists(), "failed B VM runtime is cleaned");
    assert!(!b_paths.1.exists(), "failed B cgroup is cleaned");
    assert!(!b_paths.2.exists(), "failed B materializer is cleaned");

    daemon.graceful_shutdown().await;
    wait_for_gateway_service_cleaned(pool, fixture, old_instance_id).await;
    assert!(!old_paths.0.exists(), "A VM runtime is cleaned at shutdown");
    assert!(!old_paths.1.exists(), "A cgroup is cleaned at shutdown");
    assert!(
        !old_paths.2.exists(),
        "A materializer is cleaned at shutdown"
    );
    eprintln!(
        "persistent-service-failed-candidate-passed old_instance={old_instance_id} candidate_instance={} candidate_fence={} failure_code={failure_code} old_startup_id={}",
        evidence.instance_id, evidence.fencing_token, first_proof.startup_id
    );
}

type FailedGatewayServiceRow = (
    uuid::Uuid,
    i64,
    String,
    Option<String>,
    Option<OffsetDateTime>,
    Option<i32>,
    Option<i32>,
    Option<uuid::Uuid>,
    Option<i32>,
    Option<OffsetDateTime>,
);

#[derive(Debug)]
struct FailedGatewayServiceEvidence {
    gateway_id: uuid::Uuid,
    revision_id: uuid::Uuid,
    instance_id: uuid::Uuid,
    initial_fencing_token: i64,
    fencing_token: i64,
    state: String,
    failure_code: Option<String>,
    failed_at: Option<OffsetDateTime>,
    exit_code: Option<i32>,
    exit_signal: Option<i32>,
    observed_ready: bool,
    observed_promoted: bool,
    retry: Option<(i32, Option<OffsetDateTime>)>,
}

async fn wait_for_failed_gateway_service_candidate(
    pool: &sqlx::PgPool,
    gateway_id: uuid::Uuid,
    revision_id: uuid::Uuid,
    old_identity: (uuid::Uuid, uuid::Uuid, i64),
    old_proof: &GatewayServiceRequestProof,
    public_url: &str,
) -> FailedGatewayServiceEvidence {
    let (old_instance_id, old_revision_id, old_fencing_token) = old_identity;
    tokio::time::timeout(Duration::from_secs(120), async {
        // The ownership schema has no historical ready_at column. Poll the
        // complete launch lifetime and record every observed Ready state;
        // the durable gateway event count in the caller proves that no active
        // pointer transition occurred after B was declared.
        let mut initial_fencing_token = None;
        let mut candidate_id = None;
        let mut observed_ready = false;
        let mut observed_promoted = false;
        let mut served_during_failure = false;
        loop {
            let row: Option<FailedGatewayServiceRow> = sqlx::query_as(
                "SELECT instance.id, instance.fencing_token, instance.state,
                        instance.failure_code, instance.failed_at,
                        instance.exit_code, instance.exit_signal,
                        gateway.active_revision_id,
                        retry.failure_streak, retry.next_retry_at
                   FROM gateway_service_instances AS instance
                   JOIN gateways AS gateway ON gateway.id = instance.gateway_id
                   LEFT JOIN gateway_service_retry_state AS retry
                     ON retry.gateway_id = instance.gateway_id
                    AND retry.revision_id = instance.revision_id
                  WHERE instance.gateway_id = $1
                    AND instance.revision_id = $2
                    AND (($3::uuid IS NULL AND instance.id <> $4)
                         OR instance.id = $3)
                  ORDER BY instance.created_at ASC, instance.id ASC
                  LIMIT 1",
            )
            .bind(gateway_id)
            .bind(revision_id)
            .bind(candidate_id)
            .bind(old_instance_id)
            .fetch_optional(pool)
            .await
            .expect("read failed candidate lifecycle evidence");
            if let Some((
                observed_id,
                fencing_token,
                state,
                failure_code,
                failed_at,
                exit_code,
                exit_signal,
                active_revision,
                failure_streak,
                next_retry_at,
            )) = row
            {
                if let Some(expected_id) = candidate_id {
                    assert_eq!(
                        observed_id, expected_id,
                        "failed-candidate evidence must stay bound to its first instance"
                    );
                } else {
                    candidate_id = Some(observed_id);
                    initial_fencing_token = Some(fencing_token);
                }
                observed_ready |= state == "ready";
                observed_promoted |= active_revision == Some(revision_id);
                if failure_code.is_some() && !served_during_failure {
                    let proof = exercise_gateway_service_requests(public_url).await;
                    assert_eq!(proof.startup_id, old_proof.startup_id);
                    let current_old_fence = read_gateway_service_fencing_token(
                        pool,
                        old_instance_id,
                        gateway_id,
                        old_revision_id,
                    )
                    .await;
                    assert_eq!(current_old_fence, old_fencing_token);
                    served_during_failure = true;
                }
                if let (true, Some(failure_streak)) =
                    (state == "cleaned" && failure_code.is_some(), failure_streak)
                {
                    return FailedGatewayServiceEvidence {
                        gateway_id,
                        revision_id,
                        instance_id: candidate_id.expect("capture B candidate identity"),
                        initial_fencing_token: initial_fencing_token
                            .expect("capture B initial fencing token"),
                        fencing_token,
                        state,
                        failure_code,
                        failed_at,
                        exit_code,
                        exit_signal,
                        observed_ready,
                        observed_promoted,
                        retry: Some((failure_streak, next_retry_at)),
                    };
                }
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("failed service candidate reaches durable cleanup")
}

async fn read_gateway_service_fencing_token(
    pool: &sqlx::PgPool,
    instance_id: uuid::Uuid,
    gateway_id: uuid::Uuid,
    revision_id: uuid::Uuid,
) -> i64 {
    sqlx::query_scalar(
        "SELECT fencing_token
           FROM gateway_service_instances
          WHERE id = $1 AND gateway_id = $2 AND revision_id = $3",
    )
    .bind(instance_id)
    .bind(gateway_id)
    .bind(revision_id)
    .fetch_one(pool)
    .await
    .expect("read A fencing token during failed B cleanup")
}

/// Exercises one real public revision cutover while an accepted request is
/// still executing against the old guest.  The service fixture's bounded hold
/// response keeps the exchange buffered until the new revision is serving.
// This opt-in harness function keeps the complete external cutover evidence
// together so its cleanup ordering remains reviewable at one call site.
#[allow(
    clippy::cognitive_complexity,
    clippy::too_many_arguments,
    clippy::too_many_lines
)]
async fn exercise_external_gateway_service_cutover(
    pool: &sqlx::PgPool,
    fixture: &GatewayServiceGoldenFixture,
    gateway: &GatewayEdgeConfig,
    _app_config: &AppConfig,
    _root: &Path,
    _root_image: &Path,
    release_artifact_root: &Path,
    daemon: ExternalGoldenDaemon,
    old_instance_id: uuid::Uuid,
    old_paths: (PathBuf, PathBuf, PathBuf),
    first_proof: &GatewayServiceRequestProof,
    public_url: &str,
    rollback: bool,
    candidate_capacity: bool,
) {
    let candidate =
        seed_gateway_service_cutover_candidate(pool, fixture, release_artifact_root, "/readyz")
            .await;
    let baseline_invocations: i64 =
        sqlx::query_scalar("SELECT count(*) FROM gateway_invocations WHERE gateway_id = $1")
            .bind(fixture.gateway_id)
            .fetch_one(pool)
            .await
            .expect("count service invocations before cutover hold");
    let hold_started_at: OffsetDateTime = sqlx::query_scalar("SELECT clock_timestamp()")
        .fetch_one(pool)
        .await
        .expect("read cutover hold start time");
    let old_fencing_token: i64 = sqlx::query_scalar(
        "SELECT fencing_token
           FROM gateway_service_instances
          WHERE id = $1 AND gateway_id = $2 AND revision_id = $3",
    )
    .bind(old_instance_id)
    .bind(fixture.gateway_id)
    .bind(fixture.revision_id)
    .fetch_one(pool)
    .await
    .expect("read A fencing token before hold");
    let hold_nonce = uuid::Uuid::new_v4();
    let hold_url = format!("{public_url}/gateway/service/hold?nonce={hold_nonce}");
    let hold_deadline = tokio::time::Instant::now() + Duration::from_secs(29);
    let hold_client = reqwest::Client::builder()
        .timeout(Duration::from_secs(29))
        .build()
        .expect("bounded cutover hold client");
    let hold = tokio::spawn(async move {
        let bytes = hold_client
            .get(hold_url)
            .send()
            .await
            .expect("public persistent-service hold request")
            .error_for_status()
            .expect("persistent-service hold succeeds")
            .bytes()
            .await
            .expect("read persistent-service hold response");
        let body: serde_json::Value =
            serde_json::from_slice(&bytes).expect("persistent-service hold identity JSON");
        GatewayServiceRequestProof {
            pid: body
                .get("pid")
                .and_then(serde_json::Value::as_u64)
                .expect("hold guest PID"),
            startup_id: body
                .get("startup_id")
                .and_then(serde_json::Value::as_str)
                .expect("hold guest startup identity")
                .to_owned(),
        }
    });
    let hold_invocation = wait_for_accepted_gateway_service_hold(
        pool,
        fixture,
        old_instance_id,
        old_fencing_token,
        hold_started_at,
        baseline_invocations,
    )
    .await;
    assert!(
        !hold.is_finished(),
        "A hold remains in flight after acceptance"
    );

    sqlx::query("UPDATE gateways SET desired_service_revision_id = $2 WHERE id = $1")
        .bind(fixture.gateway_id)
        .bind(candidate.revision_id)
        .execute(pool)
        .await
        .expect("declare published cutover candidate");
    let candidate_instance_id = wait_for_gateway_service_cutover_state(
        pool,
        fixture.gateway_id,
        old_instance_id,
        fixture.revision_id,
        candidate.revision_id,
    )
    .await;
    assert!(
        !hold.is_finished(),
        "A hold remains pending while B is ready, active, and A is draining"
    );
    let candidate_paths = gateway_service_resource_paths(candidate_instance_id);
    assert!(old_paths.0.is_dir(), "A VM runtime remains during drain");
    assert!(old_paths.1.is_dir(), "A cgroup remains during drain");
    assert!(old_paths.2.is_dir(), "A materializer remains during drain");
    assert!(
        candidate_paths.0.is_dir(),
        "B VM runtime exists while serving"
    );
    assert!(candidate_paths.1.is_dir(), "B cgroup exists while serving");
    assert!(
        candidate_paths.2.is_dir(),
        "B materializer exists while serving"
    );
    let b_before: OffsetDateTime = sqlx::query_scalar("SELECT clock_timestamp()")
        .fetch_one(pool)
        .await
        .expect("read B public request start time");
    let b_proof = exercise_gateway_service_cutover_requests(
        pool,
        &candidate,
        candidate_instance_id,
        public_url,
        b_before,
    )
    .await;
    assert_ne!(b_proof.startup_id, first_proof.startup_id);
    assert!(
        !hold.is_finished(),
        "A hold remains pending after B traffic"
    );
    let old_state: String =
        sqlx::query_scalar("SELECT state FROM gateway_service_instances WHERE id = $1")
            .bind(old_instance_id)
            .fetch_one(pool)
            .await
            .expect("read A state after B traffic");
    assert_eq!(old_state, "draining");
    let hold_outcome: String =
        sqlx::query_scalar("SELECT outcome FROM gateway_invocations WHERE id = $1")
            .bind(hold_invocation)
            .fetch_one(pool)
            .await
            .expect("read A hold outcome after B traffic");
    assert_eq!(hold_outcome, "accepted");
    assert!(old_paths.0.is_dir() && old_paths.1.is_dir() && old_paths.2.is_dir());

    if candidate_capacity {
        exercise_external_gateway_service_candidate_capacity(
            pool,
            fixture,
            &candidate,
            release_artifact_root,
            daemon,
            old_instance_id,
            old_fencing_token,
            old_paths,
            first_proof,
            public_url,
            candidate_instance_id,
            candidate_paths,
            &b_proof,
            hold,
            hold_invocation,
            hold_deadline,
        )
        .await;
        return;
    }

    let hold_proof = tokio::time::timeout_at(hold_deadline, hold)
        .await
        .expect("A hold completes inside the public exchange deadline")
        .expect("A hold task joins");
    assert_eq!(hold_proof.startup_id, first_proof.startup_id);
    wait_for_gateway_invocation_completed(pool, hold_invocation).await;
    wait_for_gateway_service_cleaned(pool, fixture, old_instance_id).await;
    assert!(
        !old_paths.0.exists(),
        "A VM runtime is cleaned after response"
    );
    assert!(!old_paths.1.exists(), "A cgroup is cleaned after response");
    assert!(
        !old_paths.2.exists(),
        "A materializer is cleaned after response"
    );
    assert!(
        candidate_paths.0.is_dir() && candidate_paths.1.is_dir() && candidate_paths.2.is_dir(),
        "B remains serving after A cleanup"
    );
    if rollback {
        exercise_external_gateway_service_rollback(
            pool,
            fixture,
            gateway,
            daemon,
            old_instance_id,
            &candidate,
            candidate_instance_id,
            candidate_paths,
            first_proof,
            &b_proof,
            public_url,
        )
        .await;
    } else {
        daemon.graceful_shutdown().await;
        wait_for_gateway_service_cleaned(pool, &candidate, candidate_instance_id).await;
        assert!(!candidate_paths.0.exists());
        assert!(!candidate_paths.1.exists());
        assert!(!candidate_paths.2.exists());
        let applied_config =
            wait_for_caddy_configuration(&gateway.caddy_admin_url, "/gateway/service").await;
        assert!(applied_config.contains("/gateway/service"));
        eprintln!(
            "persistent-service-cutover-passed old_instance={old_instance_id} new_instance={candidate_instance_id} old_startup_id={} new_startup_id={}",
            first_proof.startup_id, b_proof.startup_id
        );
    }
}

struct ExternalCandidateCapacitySnapshot {
    active_revision: Option<uuid::Uuid>,
    old_state: String,
    old_fencing_token: i64,
    serving_state: String,
    serving_fencing_token: i64,
    next_instances: i64,
}

async fn read_external_candidate_capacity_snapshot(
    pool: &sqlx::PgPool,
    gateway_id: uuid::Uuid,
    old_instance_id: uuid::Uuid,
    old_revision_id: uuid::Uuid,
    serving_instance_id: uuid::Uuid,
    serving_revision_id: uuid::Uuid,
    next_revision_id: uuid::Uuid,
) -> ExternalCandidateCapacitySnapshot {
    let row: (Option<uuid::Uuid>, String, i64, String, i64, i64) = sqlx::query_as(
        "SELECT gateway.active_revision_id, old_instance.state,
                old_instance.fencing_token, serving_instance.state,
                serving_instance.fencing_token, count(next_instance.id)::bigint
           FROM gateways AS gateway
           JOIN gateway_service_instances AS old_instance
             ON old_instance.id = $2
            AND old_instance.gateway_id = gateway.id
            AND old_instance.revision_id = $3
           JOIN gateway_service_instances AS serving_instance
             ON serving_instance.id = $4
            AND serving_instance.gateway_id = gateway.id
            AND serving_instance.revision_id = $5
           LEFT JOIN gateway_service_instances AS next_instance
             ON next_instance.gateway_id = gateway.id
            AND next_instance.revision_id = $6
          WHERE gateway.id = $1
          GROUP BY gateway.active_revision_id, old_instance.state,
                   old_instance.fencing_token, serving_instance.state,
                   serving_instance.fencing_token",
    )
    .bind(gateway_id)
    .bind(old_instance_id)
    .bind(old_revision_id)
    .bind(serving_instance_id)
    .bind(serving_revision_id)
    .bind(next_revision_id)
    .fetch_one(pool)
    .await
    .expect("read real candidate capacity state");
    ExternalCandidateCapacitySnapshot {
        active_revision: row.0,
        old_state: row.1,
        old_fencing_token: row.2,
        serving_state: row.3,
        serving_fencing_token: row.4,
        next_instances: row.5,
    }
}

/// Extends the real A/B cutover proof with a third revision admission check.
/// A's accepted request remains unresolved while the test observes C blocked
/// by durable capacity, then admits C only after A is physically cleaned.
#[allow(
    clippy::cognitive_complexity,
    clippy::too_many_arguments,
    clippy::too_many_lines
)]
async fn exercise_external_gateway_service_candidate_capacity(
    pool: &sqlx::PgPool,
    fixture: &GatewayServiceGoldenFixture,
    b_fixture: &GatewayServiceGoldenFixture,
    release_artifact_root: &Path,
    daemon: ExternalGoldenDaemon,
    old_instance_id: uuid::Uuid,
    old_fencing_token: i64,
    old_paths: (PathBuf, PathBuf, PathBuf),
    first_proof: &GatewayServiceRequestProof,
    public_url: &str,
    b_instance_id: uuid::Uuid,
    b_paths: (PathBuf, PathBuf, PathBuf),
    b_proof: &GatewayServiceRequestProof,
    hold: tokio::task::JoinHandle<GatewayServiceRequestProof>,
    hold_invocation: uuid::Uuid,
    hold_deadline: tokio::time::Instant,
) {
    let third =
        seed_gateway_service_cutover_candidate(pool, fixture, release_artifact_root, "/readyz")
            .await;
    let accepted_invocation: (uuid::Uuid, uuid::Uuid, uuid::Uuid, i64, String) = sqlx::query_as(
        "SELECT gateway_id, gateway_revision_id, service_instance_id,
                service_instance_fencing_token, outcome
           FROM gateway_invocations
          WHERE id = $1",
    )
    .bind(hold_invocation)
    .fetch_one(pool)
    .await
    .expect("read accepted A capacity hold");
    assert_eq!(accepted_invocation.0, fixture.gateway_id);
    assert_eq!(accepted_invocation.1, fixture.revision_id);
    assert_eq!(accepted_invocation.2, old_instance_id);
    assert_eq!(accepted_invocation.3, old_fencing_token);
    assert_eq!(accepted_invocation.4, "accepted");
    sqlx::query("UPDATE gateways SET desired_service_revision_id = $2 WHERE id = $1")
        .bind(fixture.gateway_id)
        .bind(third.revision_id)
        .execute(pool)
        .await
        .expect("declare third candidate while A drains and B serves");

    let mut observed_blocked = false;
    let mut serving_fencing_token = None;
    tokio::time::timeout_at(hold_deadline, async {
        loop {
            let snapshot = read_external_candidate_capacity_snapshot(
                pool,
                fixture.gateway_id,
                old_instance_id,
                fixture.revision_id,
                b_instance_id,
                b_fixture.revision_id,
                third.revision_id,
            )
            .await;
            assert!(
                !(snapshot.next_instances > 0 && snapshot.old_state != "cleaned"),
                "C was admitted before A cleaned: a_state={} c_instances={}",
                snapshot.old_state,
                snapshot.next_instances
            );
            assert_eq!(snapshot.old_fencing_token, old_fencing_token);
            if let Some(expected) = serving_fencing_token {
                assert_eq!(snapshot.serving_fencing_token, expected);
            } else {
                assert!(snapshot.serving_fencing_token > 0);
                serving_fencing_token = Some(snapshot.serving_fencing_token);
            }
            let hold_unresolved = !hold.is_finished();
            if hold_unresolved
                && snapshot.active_revision == Some(b_fixture.revision_id)
                && snapshot.old_state == "draining"
                && snapshot.serving_state == "ready"
                && snapshot.next_instances == 0
            {
                observed_blocked = true;
            }
            if hold.is_finished() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("A capacity hold completes inside the public exchange deadline");

    let hold_proof = hold.await.expect("A capacity hold task joins");
    assert_eq!(hold_proof.startup_id, first_proof.startup_id);

    tokio::time::timeout(Duration::from_secs(120), async {
        loop {
            let snapshot = read_external_candidate_capacity_snapshot(
                pool,
                fixture.gateway_id,
                old_instance_id,
                fixture.revision_id,
                b_instance_id,
                b_fixture.revision_id,
                third.revision_id,
            )
            .await;
            assert!(
                !(snapshot.next_instances > 0 && snapshot.old_state != "cleaned"),
                "C was admitted before A cleaned: a_state={} c_instances={}",
                snapshot.old_state,
                snapshot.next_instances
            );
            assert_eq!(snapshot.old_fencing_token, old_fencing_token);
            assert_eq!(serving_fencing_token, Some(snapshot.serving_fencing_token));
            if snapshot.old_state == "cleaned" {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("A capacity hold cleanup reaches durable Cleaned state");
    assert!(
        observed_blocked,
        "at least one durable A-draining/B-ready/C-empty snapshot is required"
    );
    wait_for_gateway_invocation_completed(pool, hold_invocation).await;
    wait_for_gateway_service_cleaned(pool, fixture, old_instance_id).await;
    assert!(!old_paths.0.exists());
    assert!(!old_paths.1.exists());
    assert!(!old_paths.2.exists());

    let third_instance_id = wait_for_gateway_service_ready(pool, &third).await;
    let third_paths = gateway_service_resource_paths(third_instance_id);
    assert!(third_paths.0.is_dir());
    assert!(third_paths.1.is_dir());
    assert!(third_paths.2.is_dir());
    let third_started_at: OffsetDateTime = sqlx::query_scalar("SELECT clock_timestamp()")
        .fetch_one(pool)
        .await
        .expect("read C request start time");
    let third_proof = exercise_gateway_service_cutover_requests(
        pool,
        &third,
        third_instance_id,
        public_url,
        third_started_at,
    )
    .await;
    assert_ne!(third_proof.startup_id, first_proof.startup_id);
    assert_ne!(third_proof.startup_id, b_proof.startup_id);

    // C promotion commits the active pointer before the paced reconciliation
    // pass asks the now-noncurrent B job to drain. B may therefore remain
    // Ready at this instant; the durable cleanup and physical-path checks
    // below prove that reconciliation eventually retires it.
    wait_for_gateway_service_cleaned(pool, b_fixture, b_instance_id).await;
    assert!(!b_paths.0.exists());
    assert!(!b_paths.1.exists());
    assert!(!b_paths.2.exists());

    daemon.graceful_shutdown().await;
    wait_for_gateway_service_cleaned(pool, &third, third_instance_id).await;
    assert!(!third_paths.0.exists());
    assert!(!third_paths.1.exists());
    assert!(!third_paths.2.exists());
    eprintln!(
        "REAL_GATEWAY_SERVICE_CANDIDATE_CAPACITY_E2E=1 a_instance={old_instance_id} b_instance={b_instance_id} c_instance={third_instance_id} a_startup_id={} b_startup_id={} c_startup_id={}",
        first_proof.startup_id, b_proof.startup_id, third_proof.startup_id
    );
}

/// Re-selects the original immutable release after B has served. A finite B
/// hold keeps the old candidate draining long enough to prove the replacement
/// becomes Ready and active before B is retired.
// Keep the complete operator rollback evidence at one call site so its
// ordering and resource assertions remain reviewable together.
#[allow(
    clippy::cognitive_complexity,
    clippy::too_many_arguments,
    clippy::too_many_lines
)]
async fn exercise_external_gateway_service_rollback(
    pool: &sqlx::PgPool,
    fixture: &GatewayServiceGoldenFixture,
    gateway: &GatewayEdgeConfig,
    daemon: ExternalGoldenDaemon,
    original_a_instance_id: uuid::Uuid,
    b_fixture: &GatewayServiceGoldenFixture,
    b_instance_id: uuid::Uuid,
    b_paths: (PathBuf, PathBuf, PathBuf),
    first_a_proof: &GatewayServiceRequestProof,
    b_proof: &GatewayServiceRequestProof,
    public_url: &str,
) {
    let baseline_invocations: i64 =
        sqlx::query_scalar("SELECT count(*) FROM gateway_invocations WHERE gateway_id = $1")
            .bind(fixture.gateway_id)
            .fetch_one(pool)
            .await
            .expect("count service invocations before rollback hold");
    let hold_started_at: OffsetDateTime = sqlx::query_scalar("SELECT clock_timestamp()")
        .fetch_one(pool)
        .await
        .expect("read rollback hold start time");
    let b_fencing_token: i64 = sqlx::query_scalar(
        "SELECT fencing_token
           FROM gateway_service_instances
          WHERE id = $1 AND gateway_id = $2 AND revision_id = $3",
    )
    .bind(b_instance_id)
    .bind(b_fixture.gateway_id)
    .bind(b_fixture.revision_id)
    .fetch_one(pool)
    .await
    .expect("read B fencing token before rollback hold");
    let hold_nonce = uuid::Uuid::new_v4();
    let hold_url = format!("{public_url}/gateway/service/hold?rollback_nonce={hold_nonce}");
    let hold_deadline = tokio::time::Instant::now() + Duration::from_secs(29);
    let hold_client = reqwest::Client::builder()
        .timeout(Duration::from_secs(29))
        .build()
        .expect("bounded rollback hold client");
    let hold = tokio::spawn(async move {
        let bytes = hold_client
            .get(hold_url)
            .send()
            .await
            .expect("public rollback hold request")
            .error_for_status()
            .expect("rollback hold succeeds")
            .bytes()
            .await
            .expect("read rollback hold response");
        let body: serde_json::Value =
            serde_json::from_slice(&bytes).expect("rollback hold identity JSON");
        GatewayServiceRequestProof {
            pid: body
                .get("pid")
                .and_then(serde_json::Value::as_u64)
                .expect("rollback hold guest PID"),
            startup_id: body
                .get("startup_id")
                .and_then(serde_json::Value::as_str)
                .expect("rollback hold guest startup identity")
                .to_owned(),
        }
    });
    let hold_invocation = wait_for_accepted_gateway_service_hold(
        pool,
        b_fixture,
        b_instance_id,
        b_fencing_token,
        hold_started_at,
        baseline_invocations,
    )
    .await;
    assert!(
        !hold.is_finished(),
        "B hold remains in flight after acceptance"
    );

    sqlx::query("UPDATE gateways SET desired_service_revision_id = $2 WHERE id = $1")
        .bind(fixture.gateway_id)
        .bind(fixture.revision_id)
        .execute(pool)
        .await
        .expect("declare rollback to original A revision");
    let rollback_a_instance_id = wait_for_gateway_service_cutover_state(
        pool,
        fixture.gateway_id,
        b_instance_id,
        b_fixture.revision_id,
        fixture.revision_id,
    )
    .await;
    assert_ne!(rollback_a_instance_id, original_a_instance_id);
    assert_ne!(rollback_a_instance_id, b_instance_id);
    assert!(
        !hold.is_finished(),
        "B hold remains pending during rollback"
    );
    let rollback_a_paths = gateway_service_resource_paths(rollback_a_instance_id);
    assert!(
        b_paths.0.is_dir(),
        "B VM runtime remains during rollback drain"
    );
    assert!(b_paths.1.is_dir(), "B cgroup remains during rollback drain");
    assert!(
        b_paths.2.is_dir(),
        "B materializer remains during rollback drain"
    );
    assert!(
        rollback_a_paths.0.is_dir(),
        "rollback A VM runtime is serving"
    );
    assert!(rollback_a_paths.1.is_dir(), "rollback A cgroup is serving");
    assert!(
        rollback_a_paths.2.is_dir(),
        "rollback A materializer is serving"
    );
    let a_before: OffsetDateTime = sqlx::query_scalar("SELECT clock_timestamp()")
        .fetch_one(pool)
        .await
        .expect("read rollback A public request start time");
    let rollback_a_proof = exercise_gateway_service_cutover_requests(
        pool,
        fixture,
        rollback_a_instance_id,
        public_url,
        a_before,
    )
    .await;
    assert_ne!(rollback_a_proof.startup_id, first_a_proof.startup_id);
    assert_ne!(rollback_a_proof.startup_id, b_proof.startup_id);
    assert!(
        !hold.is_finished(),
        "B hold remains pending after rollback A traffic"
    );
    let b_state: String = sqlx::query_scalar(
        "SELECT state FROM gateway_service_instances
           WHERE id = $1 AND gateway_id = $2 AND revision_id = $3",
    )
    .bind(b_instance_id)
    .bind(b_fixture.gateway_id)
    .bind(b_fixture.revision_id)
    .fetch_one(pool)
    .await
    .expect("read B state after rollback A traffic");
    assert_eq!(b_state, "draining");
    let hold_outcome: String =
        sqlx::query_scalar("SELECT outcome FROM gateway_invocations WHERE id = $1")
            .bind(hold_invocation)
            .fetch_one(pool)
            .await
            .expect("read B rollback hold outcome");
    assert_eq!(hold_outcome, "accepted");

    let hold_proof = tokio::time::timeout_at(hold_deadline, hold)
        .await
        .expect("B hold completes inside the public exchange deadline")
        .expect("B hold task joins");
    assert_eq!(hold_proof.startup_id, b_proof.startup_id);
    wait_for_gateway_invocation_completed(pool, hold_invocation).await;
    wait_for_gateway_service_cleaned(pool, b_fixture, b_instance_id).await;
    assert!(!b_paths.0.exists());
    assert!(!b_paths.1.exists());
    assert!(!b_paths.2.exists());
    assert!(rollback_a_paths.0.is_dir() && rollback_a_paths.1.is_dir());
    assert!(rollback_a_paths.2.is_dir());

    daemon.graceful_shutdown().await;
    wait_for_gateway_service_cleaned(pool, fixture, rollback_a_instance_id).await;
    assert!(!rollback_a_paths.0.exists());
    assert!(!rollback_a_paths.1.exists());
    assert!(!rollback_a_paths.2.exists());
    let applied_config =
        wait_for_caddy_configuration(&gateway.caddy_admin_url, "/gateway/service").await;
    assert!(applied_config.contains("/gateway/service"));
    eprintln!(
        "persistent-service-rollback-passed original_a_instance={original_a_instance_id} b_instance={b_instance_id} rollback_a_instance={rollback_a_instance_id} original_a_startup_id={} b_startup_id={} rollback_a_startup_id={}",
        first_a_proof.startup_id, b_proof.startup_id, rollback_a_proof.startup_id
    );
}

async fn wait_for_accepted_gateway_service_hold(
    pool: &sqlx::PgPool,
    fixture: &GatewayServiceGoldenFixture,
    instance_id: uuid::Uuid,
    fencing_token: i64,
    started_at: OffsetDateTime,
    baseline_invocations: i64,
) -> uuid::Uuid {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let count: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM gateway_invocations WHERE gateway_id = $1",
            )
            .bind(fixture.gateway_id)
            .fetch_one(pool)
            .await
            .expect("count accepted hold invocation");
            let row: Option<(uuid::Uuid, i64)> = sqlx::query_as(
                "SELECT invocation.id, invocation.service_instance_fencing_token
                   FROM gateway_invocations AS invocation
                   JOIN gateway_runtime_authority_sessions AS session
                     ON session.invocation_id = invocation.id
                    AND session.gateway_id = invocation.gateway_id
                    AND session.gateway_revision_id = invocation.gateway_revision_id
                  WHERE invocation.gateway_id = $1
                    AND invocation.gateway_revision_id = $2
                    AND invocation.service_instance_id = $3
                    AND invocation.outcome = 'accepted'
                    AND invocation.accepted_at >= $4
                    AND session.admission_mode = 'host_mediated'
                    AND session.status = 'active'
                  ORDER BY invocation.accepted_at DESC, invocation.id DESC
                  LIMIT 1",
            )
            .bind(fixture.gateway_id)
            .bind(fixture.revision_id)
            .bind(instance_id)
            .bind(started_at)
            .fetch_optional(pool)
            .await
            .expect("read accepted host-mediated hold invocation");
            if count == baseline_invocations + 1 {
                if let Some((invocation_id, observed_fencing_token)) = row {
                    assert_eq!(observed_fencing_token, fencing_token);
                    return invocation_id;
                }
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("A hold becomes an accepted host-mediated invocation")
}

async fn wait_for_gateway_service_cutover_state(
    pool: &sqlx::PgPool,
    gateway_id: uuid::Uuid,
    old_instance_id: uuid::Uuid,
    old_revision_id: uuid::Uuid,
    candidate_revision_id: uuid::Uuid,
) -> uuid::Uuid {
    tokio::time::timeout(Duration::from_secs(18), async {
        loop {
            let row: Option<(Option<uuid::Uuid>, String, uuid::Uuid, String)> = sqlx::query_as(
                "SELECT gateway.active_revision_id, old_instance.state,
                        candidate.id, candidate.state
                   FROM gateways AS gateway
                   JOIN gateway_service_instances AS old_instance
                     ON old_instance.id = $2
                    AND old_instance.gateway_id = gateway.id
                    AND old_instance.revision_id = $3
                   JOIN gateway_service_instances AS candidate
                     ON candidate.gateway_id = gateway.id
                    AND candidate.revision_id = $4
                  WHERE gateway.id = $1
                    AND candidate.state = 'ready'
                  ORDER BY candidate.created_at DESC
                  LIMIT 1",
            )
            .bind(gateway_id)
            .bind(old_instance_id)
            .bind(old_revision_id)
            .bind(candidate_revision_id)
            .fetch_optional(pool)
            .await
            .expect("read coherent service cutover state");
            if let Some((Some(active), old_state, candidate_id, candidate_state)) = row {
                if active == candidate_revision_id
                    && old_state == "draining"
                    && candidate_state == "ready"
                {
                    return candidate_id;
                }
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("B becomes active while A drains")
}

async fn exercise_gateway_service_cutover_requests(
    pool: &sqlx::PgPool,
    candidate: &GatewayServiceGoldenFixture,
    candidate_instance_id: uuid::Uuid,
    public_url: &str,
    started_at: OffsetDateTime,
) -> GatewayServiceRequestProof {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("bounded B cutover client");
    let first_nonce = uuid::Uuid::new_v4();
    let second_nonce = uuid::Uuid::new_v4();
    let path = format!("{public_url}/gateway/service/identity");
    let first = client
        .get(format!("{path}?cutover_nonce={first_nonce}"))
        .send()
        .await
        .expect("first B public identity request")
        .error_for_status()
        .expect("first B identity succeeds")
        .bytes()
        .await
        .expect("read first B identity");
    let second = client
        .get(format!("{path}?cutover_nonce={second_nonce}"))
        .send()
        .await
        .expect("second B public identity request")
        .error_for_status()
        .expect("second B identity succeeds")
        .bytes()
        .await
        .expect("read second B identity");
    let first: serde_json::Value = serde_json::from_slice(&first).expect("first B identity JSON");
    let second: serde_json::Value =
        serde_json::from_slice(&second).expect("second B identity JSON");
    let first_startup = first
        .get("startup_id")
        .and_then(serde_json::Value::as_str)
        .expect("first B startup identity");
    let second_startup = second
        .get("startup_id")
        .and_then(serde_json::Value::as_str)
        .expect("second B startup identity");
    assert_eq!(first_startup, second_startup);
    assert!(
        second
            .get("request_count")
            .and_then(serde_json::Value::as_u64)
            .expect("second B request count")
            > first
                .get("request_count")
                .and_then(serde_json::Value::as_u64)
                .expect("first B request count")
    );
    // The nonce values make the two public URLs distinct.  Invocation rows do
    // not persist query strings, so the DB correlation is the exact two-row
    // delta after the exclusive cutover request window.
    let candidate_fence: i64 =
        sqlx::query_scalar("SELECT fencing_token FROM gateway_service_instances WHERE id = $1")
            .bind(candidate_instance_id)
            .fetch_one(pool)
            .await
            .expect("read B fencing token");
    let bindings: Vec<(uuid::Uuid, Option<uuid::Uuid>, Option<i64>, String)> = sqlx::query_as(
        "SELECT gateway_revision_id, service_instance_id,
                service_instance_fencing_token, outcome
           FROM gateway_invocations
          WHERE gateway_id = $1 AND accepted_at >= $2
          ORDER BY accepted_at DESC, id DESC
          LIMIT 3",
    )
    .bind(candidate.gateway_id)
    .bind(started_at)
    .fetch_all(pool)
    .await
    .expect("read B invocation bindings");
    assert_eq!(bindings.len(), 2);
    assert!(
        bindings
            .iter()
            .all(|(revision, instance, fencing_token, outcome)| {
                *revision == candidate.revision_id
                    && *instance == Some(candidate_instance_id)
                    && *fencing_token == Some(candidate_fence)
                    && outcome == "completed"
            },)
    );
    GatewayServiceRequestProof {
        pid: first
            .get("pid")
            .and_then(serde_json::Value::as_u64)
            .expect("B guest PID"),
        startup_id: first_startup.to_owned(),
    }
}

async fn wait_for_gateway_invocation_completed(pool: &sqlx::PgPool, invocation_id: uuid::Uuid) {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let outcome: String =
                sqlx::query_scalar("SELECT outcome FROM gateway_invocations WHERE id = $1")
                    .bind(invocation_id)
                    .fetch_one(pool)
                    .await
                    .expect("read completed hold invocation");
            if outcome == "completed" {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("A hold invocation completes");
}

struct ExternalGoldenDaemon {
    child: tokio::process::Child,
    base_url: String,
    log_path: PathBuf,
}

impl Drop for ExternalGoldenDaemon {
    fn drop(&mut self) {
        if self.child.id().is_some() {
            let _ = self.child.start_kill();
        }
    }
}

impl ExternalGoldenDaemon {
    async fn graceful_shutdown(mut self) {
        let pid = self.child.id().expect("external daemon child PID");
        let status = Command::new("kill")
            .args(["-INT", &pid.to_string()])
            .status()
            .await
            .expect("send external daemon Ctrl-C");
        assert!(status.success(), "send external daemon Ctrl-C succeeds");
        let status = tokio::time::timeout(Duration::from_secs(45), self.child.wait())
            .await
            .expect("external daemon graceful shutdown timeout")
            .expect("wait for external daemon graceful shutdown");
        assert!(
            status.success(),
            "external daemon exits successfully after Ctrl-C; log={} ",
            self.log_path.display()
        );
    }

    async fn unclean_kill(mut self) -> std::process::ExitStatus {
        let pid = self.child.id().expect("external daemon child PID");
        let status = Command::new("kill")
            .args(["-KILL", &pid.to_string()])
            .status()
            .await
            .expect("send external daemon SIGKILL");
        assert!(status.success(), "send external daemon SIGKILL succeeds");
        tokio::time::timeout(Duration::from_secs(30), self.child.wait())
            .await
            .expect("external daemon SIGKILL wait timeout")
            .expect("wait for external daemon SIGKILL")
    }
}

// This fixture must spell out the production environment contract so the
// child cannot accidentally inherit the in-process AppConfig implementation.
#[allow(clippy::too_many_lines)]
async fn spawn_external_golden_daemon(
    gateway: &GatewayEdgeConfig,
    app_config: &AppConfig,
    root: &Path,
    root_image: &Path,
    log_path_override: Option<&Path>,
) -> ExternalGoldenDaemon {
    let http_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("reserve external daemon HTTP listener");
    let http_addr = http_listener
        .local_addr()
        .expect("external daemon HTTP listener address");
    drop(http_listener);

    let secret_key_directory = root.join("external-secret-keys");
    std::fs::create_dir_all(&secret_key_directory).expect("create external secret key directory");
    std::fs::set_permissions(
        &secret_key_directory,
        std::fs::Permissions::from_mode(0o700),
    )
    .expect("set external secret key directory mode");
    // The production loader treats the filename as the key reference; keep
    // the slash-free reference used by the in-process golden provider.
    let secret_key_path = secret_key_directory.join("golden-v1");
    if !secret_key_path.exists() {
        std::fs::write(&secret_key_path, [17_u8; 32]).expect("write external secret key");
        std::fs::set_permissions(&secret_key_path, std::fs::Permissions::from_mode(0o400))
            .expect("set external secret key mode");
    }

    let handoff_key_path = root.join("external-runtime-authority.key");
    if !handoff_key_path.exists() {
        std::fs::write(&handoff_key_path, app_config.runtime_authority_handoff_key)
            .expect("write external runtime authority key");
        std::fs::set_permissions(&handoff_key_path, std::fs::Permissions::from_mode(0o400))
            .expect("set external runtime authority key mode");
    }
    let callback_path = root.join("external-registry-callback-token");
    if !callback_path.exists() {
        std::fs::write(
            &callback_path,
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        )
        .expect("write external registry callback token");
    }
    let registry_key_path = root.join("external-registry-token-private.pem");
    if !registry_key_path.exists() {
        let registry_key_status = Command::new("openssl")
            .args([
                "genpkey",
                "-algorithm",
                "RSA",
                "-pkeyopt",
                "rsa_keygen_bits:2048",
                "-out",
                registry_key_path.to_str().expect("registry key path UTF-8"),
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .expect("launch openssl for external registry key");
        assert!(
            registry_key_status.success(),
            "generate external registry key"
        );
        std::fs::set_permissions(&registry_key_path, std::fs::Permissions::from_mode(0o400))
            .expect("set external registry key mode");
    }

    let root_manifest_path = root.join("external-root-image-manifest.json");
    if !root_manifest_path.exists() {
        std::fs::write(
            &root_manifest_path,
            serde_json::json!({
                "version": 1,
                "roots": {
                    ROOT_IMAGE: { "kind": "directory", "path": root_image }
                }
            })
            .to_string(),
        )
        .expect("write external root image manifest");
    }
    let caddy_config_path = root.join("external-caddy-config.json");
    if !caddy_config_path.exists() {
        std::fs::write(&caddy_config_path, &gateway.caddy_configuration_template)
            .expect("write external Caddy configuration template");
    }
    let log_path = log_path_override.map_or_else(
        || {
            env::var_os("HEPHAESTUS_EXTERNAL_DAEMON_LOG")
                .map_or_else(|| root.join("external-hephaestusd.log"), PathBuf::from)
        },
        Path::to_path_buf,
    );
    let log = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&log_path)
        .expect("create external daemon log");
    let stderr = log.try_clone().expect("clone external daemon log");
    let runtime_root = env::var("HEPHAESTUS_LIBKRUN_RUNTIME_ROOT")
        .expect("libkrun runtime root for external daemon");
    let cgroup_root = env::var("HEPHAESTUS_LIBKRUN_CGROUP_ROOT")
        .expect("libkrun cgroup root for external daemon");
    let worker = env::var("HEPHAESTUS_LIBKRUN_WORKER").expect("libkrun worker for external daemon");
    let daemon_binary = env::var_os("HEPHAESTUS_DAEMON_BINARY")
        .map(PathBuf::from)
        .or_else(|| option_env!("CARGO_BIN_EXE_hephaestusd").map(PathBuf::from))
        .expect("external daemon binary path must be supplied by the harness");
    assert!(
        daemon_binary.is_file(),
        "external daemon binary must be built at {}",
        daemon_binary.display()
    );
    let registry_service = "registry.golden.invalid";
    let mut command = Command::new(&daemon_binary);
    command
        .env("HEPHAESTUS_DATABASE_URL", &app_config.database_url)
        .env("HEPHAESTUS_NATS_URL", &app_config.nats_url)
        .env("HEPHAESTUS_HTTP_LISTEN", http_addr.to_string())
        .env("HEPHAESTUS_REPOSITORY_ROOT", &app_config.repository_root)
        .env(
            "HEPHAESTUS_GIT_HTTP_BACKEND",
            app_config
                .git_http_backend
                .to_str()
                .expect("Git HTTP backend path UTF-8"),
        )
        .env(
            "HEPHAESTUS_GIT_PRE_RECEIVE_HOOK",
            &app_config.git_pre_receive_hook,
        )
        .env("HEPHAESTUS_OIDC_ISSUER", golden_issuer())
        .env("HEPHAESTUS_OIDC_AUDIENCE", AUDIENCE)
        .env("HEPHAESTUS_OIDC_ALGORITHM", "HS256")
        .env(
            "HEPHAESTUS_OIDC_HS256_SECRET",
            std::str::from_utf8(SIGNING_SECRET).expect("golden signing secret UTF-8"),
        )
        .env("HEPHAESTUS_VOLUME_ROOT", &app_config.volumes.volume_root)
        .env(
            "HEPHAESTUS_WORKSPACE_ROOT",
            &app_config.workspaces.workspace_root,
        )
        .env(
            "HEPHAESTUS_ARTIFACT_ROOT",
            &app_config.workspaces.artifact_root,
        )
        .env("HEPHAESTUS_RUNTIME_ROOT", runtime_root)
        .env(
            "HEPHAESTUS_SECRET_RUNTIME_ROOT",
            &app_config.secret_mounts.root,
        )
        .env("HEPHAESTUS_SECRET_KEY_DIRECTORY", &secret_key_directory)
        .env("HEPHAESTUS_SECRET_KEY_REFERENCE", "golden-v1")
        .env(
            "HEPHAESTUS_RUNTIME_AUTHORITY_HANDOFF_KEY_FILE",
            &handoff_key_path,
        )
        .env(
            "HEPHAESTUS_RPC_MEDIATOR_SECRET",
            "golden-external-rpc-secret",
        )
        .env("HEPHAESTUS_REGISTRY_TOKEN_PRIVATE_KEY", &registry_key_path)
        .env(
            "HEPHAESTUS_REGISTRY_TOKEN_ISSUER",
            "http://127.0.0.1:1/v1/registry/token",
        )
        .env("HEPHAESTUS_REGISTRY_SERVICE", registry_service)
        .env("HEPHAESTUS_REGISTRY_PRIVATE_ORIGIN", "http://127.0.0.1:1/")
        .env("HEPHAESTUS_REGISTRY_TOKEN_KEY_ID", "golden-external-v1")
        .env("HEPHAESTUS_REGISTRY_TOKEN_LIFETIME_SECONDS", "300")
        .env(
            "HEPHAESTUS_REGISTRY_NOTIFICATION_CALLBACK_TOKEN_FILE",
            &callback_path,
        )
        .env("HEPHAESTUS_ROOT_IMAGE_MANIFEST", &root_manifest_path)
        .env_remove("HEPHAESTUS_ROOT_IMAGE_PATH")
        .env_remove("HEPHAESTUS_ROOT_IMAGE_REFERENCE")
        .env("HEPHAESTUS_VM_BACKEND", "libkrun")
        .env("HEPHAESTUS_LIBKRUN_WORKER", worker)
        .env("HEPHAESTUS_CGROUP_ROOT", cgroup_root)
        .env("HEPHAESTUS_HOST_ID", &app_config.volumes.host_id)
        .env("HEPHAESTUS_MKFS_EXT4", mkfs_ext4())
        .env(
            "HEPHAESTUS_RUNTIME_POLICY_VERSION",
            &app_config.runtime_policy.version,
        )
        .env(
            "HEPHAESTUS_RUNTIME_MAX_VCPUS",
            app_config.runtime_policy.max_vcpus.to_string(),
        )
        .env(
            "HEPHAESTUS_RUNTIME_MAX_MEMORY_MIB",
            app_config.runtime_policy.max_memory_mib.to_string(),
        )
        .env(
            "HEPHAESTUS_RUNTIME_ALLOW_BROKER_ONLY",
            app_config.runtime_policy.allow_broker_only.to_string(),
        )
        .env(
            "HEPHAESTUS_RUNTIME_ALLOW_EGRESS",
            app_config.runtime_policy.allow_egress.to_string(),
        )
        .env(
            "HEPHAESTUS_SECRET_BROKER_SOCKET",
            &app_config.secret_broker_socket,
        )
        .env(
            "HEPHAESTUS_RUNTIME_AUTHORITY_SESSION_TTL_SECONDS",
            app_config
                .runtime_authority_session_ttl
                .as_secs()
                .to_string(),
        )
        .env("HEPHAESTUS_CADDY_ADMIN_URL", &gateway.caddy_admin_url)
        .env(
            "HEPHAESTUS_GATEWAY_DISPATCHER_LISTEN",
            gateway.dispatcher_listen.to_string(),
        )
        .env(
            "HEPHAESTUS_GATEWAY_PUBLIC_AUTHORITY",
            &gateway.public_authority,
        )
        .env("HEPHAESTUS_CADDY_CONFIGURATION_FILE", &caddy_config_path)
        .env("HEPHAESTUS_CADDY_SERVER_NAME", &gateway.caddy_server_name)
        .env(
            "RUST_LOG",
            "hephaestus_app=debug,gateway_edge=debug,gateway_postgres=debug,vm_libkrun=debug,run_runtime_local=debug",
        );
    if let Ok(library) = env::var("HEPHAESTUS_LIBKRUN_LIBRARY") {
        command.env("HEPHAESTUS_LIBKRUN_LIBRARY", library);
    }
    let child = command
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(stderr))
        .kill_on_drop(true)
        .spawn()
        .expect("spawn external hephaestusd");
    ExternalGoldenDaemon {
        child,
        base_url: format!("http://{http_addr}"),
        log_path,
    }
}

async fn wait_for_external_daemon_health(daemon: &mut ExternalGoldenDaemon) {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .expect("external daemon health client");
    tokio::time::timeout(Duration::from_secs(120), async {
        loop {
            if let Some(status) = daemon.child.try_wait().expect("poll external daemon child") {
                let log = std::fs::read_to_string(&daemon.log_path)
                    .unwrap_or_else(|error| format!("unable to read child log: {error}"));
                panic!(
                    "external hephaestusd exited before healthz: {status}; log={}\\n{}",
                    daemon.log_path.display(),
                    log
                );
            }
            if client
                .get(format!("{}/healthz", daemon.base_url))
                .send()
                .await
                .is_ok_and(|response| response.status().is_success())
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .unwrap_or_else(|_| {
        panic!(
            "external hephaestusd health timeout; log={} ",
            daemon.log_path.display()
        )
    });
}

fn gateway_service_resource_paths(instance_id: uuid::Uuid) -> (PathBuf, PathBuf, PathBuf) {
    let vm_id = format!("gateway-service-{instance_id}");
    let runtime_root = PathBuf::from(
        env::var("HEPHAESTUS_LIBKRUN_RUNTIME_ROOT")
            .expect("libkrun runtime root for service cleanup assertion"),
    );
    let cgroup_root = PathBuf::from(
        env::var("HEPHAESTUS_LIBKRUN_CGROUP_ROOT")
            .expect("libkrun cgroup root for service cleanup assertion"),
    );
    (
        runtime_root.join(&vm_id),
        cgroup_root.join(&vm_id),
        runtime_root
            .join("exact-runs")
            .join("gateway-services")
            .join(instance_id.to_string()),
    )
}

type GatewayServiceOwnership = (String, uuid::Uuid, i64, OffsetDateTime);

struct GatewayServiceRecoverySnapshot {
    old_state: String,
    old_host: String,
    old_owner: uuid::Uuid,
    old_fence: i64,
    old_lease: OffsetDateTime,
    old_cleaned_at: Option<OffsetDateTime>,
    active_revision: Option<uuid::Uuid>,
    candidate_count: i64,
    candidate_id: Option<uuid::Uuid>,
    candidate_state: Option<String>,
    candidate_host: Option<String>,
    candidate_owner: Option<uuid::Uuid>,
    candidate_fence: Option<i64>,
    candidate_created_at: Option<OffsetDateTime>,
    db_now: OffsetDateTime,
}

async fn read_gateway_service_ownership(
    pool: &sqlx::PgPool,
    instance_id: uuid::Uuid,
) -> GatewayServiceOwnership {
    sqlx::query_as(
        "SELECT owner_host_id, owner_uuid, fencing_token, lease_expires_at
           FROM gateway_service_instances
          WHERE id = $1",
    )
    .bind(instance_id)
    .fetch_one(pool)
    .await
    .expect("read gateway service ownership")
}

async fn read_gateway_service_recovery_snapshot(
    pool: &sqlx::PgPool,
    fixture: &GatewayServiceGoldenFixture,
    old_instance_id: uuid::Uuid,
    recovery_started_at: OffsetDateTime,
) -> GatewayServiceRecoverySnapshot {
    let row = sqlx::query(
        r"
    SELECT old.state AS old_state,
           old.owner_host_id AS old_host,
           old.owner_uuid AS old_owner,
           old.fencing_token AS old_fence,
           old.lease_expires_at AS old_lease,
           old.cleaned_at AS old_cleaned_at,
           gateway.active_revision_id AS active_revision,
           (SELECT count(*)
              FROM gateway_service_instances historical
             WHERE historical.gateway_id = old.gateway_id
               AND historical.revision_id = old.revision_id
               AND historical.id <> old.id
               AND historical.created_at >= $4) AS candidate_count,
           candidate.id AS candidate_id,
           candidate.state AS candidate_state,
           candidate.owner_host_id AS candidate_host,
           candidate.owner_uuid AS candidate_owner,
           candidate.fencing_token AS candidate_fence,
           candidate.created_at AS candidate_created_at,
           clock_timestamp() AS db_now
      FROM gateway_service_instances old
      JOIN gateways gateway ON gateway.id = old.gateway_id
 LEFT JOIN LATERAL (
           SELECT id, state, owner_host_id, owner_uuid, fencing_token, created_at
             FROM gateway_service_instances current_instance
            WHERE current_instance.gateway_id = old.gateway_id
              AND current_instance.revision_id = old.revision_id
              AND current_instance.id <> old.id
              AND current_instance.created_at >= $4
              AND current_instance.state <> 'cleaned'
            ORDER BY created_at DESC, id DESC
            LIMIT 1
      ) candidate ON TRUE
     WHERE old.id = $1
       AND old.gateway_id = $2
       AND old.revision_id = $3
",
    )
    .bind(old_instance_id)
    .bind(fixture.gateway_id)
    .bind(fixture.revision_id)
    .bind(recovery_started_at)
    .fetch_one(pool)
    .await
    .expect("read coherent boot recovery snapshot");
    GatewayServiceRecoverySnapshot {
        old_state: row.get("old_state"),
        old_host: row.get("old_host"),
        old_owner: row.get("old_owner"),
        old_fence: row.get("old_fence"),
        old_lease: row.get("old_lease"),
        old_cleaned_at: row.get("old_cleaned_at"),
        active_revision: row.get("active_revision"),
        candidate_count: row.get("candidate_count"),
        candidate_id: row.get("candidate_id"),
        candidate_state: row.get("candidate_state"),
        candidate_host: row.get("candidate_host"),
        candidate_owner: row.get("candidate_owner"),
        candidate_fence: row.get("candidate_fence"),
        candidate_created_at: row.get("candidate_created_at"),
        db_now: row.get("db_now"),
    }
}

async fn wait_for_gateway_service_cleaned(
    pool: &sqlx::PgPool,
    fixture: &GatewayServiceGoldenFixture,
    instance_id: uuid::Uuid,
) {
    tokio::time::timeout(Duration::from_secs(120), async {
        loop {
            let state: Option<String> = sqlx::query_scalar(
                "SELECT state FROM gateway_service_instances
                   WHERE id = $1 AND gateway_id = $2 AND revision_id = $3",
            )
            .bind(instance_id)
            .bind(fixture.gateway_id)
            .bind(fixture.revision_id)
            .fetch_optional(pool)
            .await
            .expect("read gateway service cleanup state");
            if state.as_deref() == Some("cleaned") {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("gateway service reaches durable Cleaned state");
}

async fn wait_for_gateway_service_boot_replacement(
    pool: &sqlx::PgPool,
    fixture: &GatewayServiceGoldenFixture,
    old_instance_id: uuid::Uuid,
    old_ownership: GatewayServiceOwnership,
    old_paths: (PathBuf, PathBuf, PathBuf),
    recovery_started_at: OffsetDateTime,
) -> uuid::Uuid {
    tokio::time::timeout(Duration::from_secs(120), async {
        let mut observed_unexpired_empty = false;
        loop {
            let snapshot = read_gateway_service_recovery_snapshot(
                pool,
                fixture,
                old_instance_id,
                recovery_started_at,
            )
            .await;

            if snapshot.db_now < old_ownership.3 {
                assert_eq!(
                    snapshot.candidate_count, 0,
                    "no replacement attempt may be admitted while the old lease is live"
                );
                observed_unexpired_empty = true;
            }
            if let Some(candidate_id) = snapshot.candidate_id {
                assert!(
                    snapshot.db_now >= old_ownership.3,
                    "a replacement service must not be admitted before the old lease expires"
                );
                assert_eq!(
                    snapshot.old_state, "cleaned",
                    "old service must be durably cleaned before replacement admission"
                );
                assert!(
                    snapshot.old_lease >= old_ownership.3,
                    "recovery must renew the fenced old claim from the original lease"
                );
                assert_eq!(snapshot.old_host, old_ownership.0);
                assert_eq!(
                    snapshot.candidate_host.as_deref(),
                    Some(snapshot.old_host.as_str())
                );
                assert_ne!(snapshot.old_owner, old_ownership.1);
                assert!(snapshot.old_fence > old_ownership.2);
                assert_eq!(snapshot.candidate_owner, Some(snapshot.old_owner));
                assert_ne!(snapshot.candidate_owner, Some(old_ownership.1));
                assert!(snapshot.candidate_fence.is_some_and(|fence| fence > 0));
                assert!(
                    snapshot
                        .candidate_created_at
                        .expect("candidate creation timestamp")
                        >= old_ownership.3,
                    "replacement creation must follow old lease expiry"
                );
                assert!(
                    snapshot
                        .candidate_created_at
                        .expect("candidate creation timestamp")
                        >= snapshot.old_cleaned_at.expect("old cleanup timestamp"),
                    "replacement creation must follow old durable cleanup"
                );
                assert!(
                    !old_paths.0.exists(),
                    "old VM runtime must be gone before Ready"
                );
                assert!(
                    !old_paths.1.exists(),
                    "old cgroup must be gone before Ready"
                );
                assert!(
                    !old_paths.2.exists(),
                    "old materializer must be gone before Ready"
                );
                if snapshot.candidate_state.as_deref() == Some("ready")
                    && snapshot.active_revision == Some(fixture.revision_id)
                {
                    assert!(
                        observed_unexpired_empty,
                        "must observe an unexpired window with no replacement rows"
                    );
                    return candidate_id;
                }
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("daemon boot recovery replaces the abandoned service")
}

type MailboxTimeoutEvidence = (
    i32,
    uuid::Uuid,
    String,
    String,
    String,
    Option<String>,
    Option<String>,
);
const GOLDEN_AGENT: &str = r#"#!/bin/sh
  set -eu
  if test -r /run/hephaestus/mailbox-event.json; then
      if grep -q '"route":"/gateway/golden-proof"' /run/hephaestus/mailbox-event.json; then
          test "$(cat /run/hephaestus/mailbox-body)" = "gateway-real-mailbox-body"
      else
          grep -q '"route":"/mailbox/golden-proof"' /run/hephaestus/mailbox-event.json
          test "$(cat /run/hephaestus/mailbox-body)" = "golden-real-mailbox-body"
      fi
      printf 'mailbox-body-ok\n' > /var/lib/hephaestus/golden-state
      exit 0
  fi
  if test -x /usr/libexec/hephaestus/integration-check \
      && test -r /run/hephaestus-secrets/.runtime-credential; then
      exec /usr/libexec/hephaestus/integration-check --brokered-https-e2e
  fi
  test -r /workspace/repo/input.txt
  test -r /run/hephaestus/parameters.json
  test -r /run/hephaestus/context.json
  if printf 'forbidden\n' > /release/write-must-fail 2>/dev/null; then
      exit 91
  fi
  if printf 'forbidden\n' > /workspace/repo/write-must-fail 2>/dev/null; then
      exit 92
  fi
  if printf 'forbidden\n' > /run/hephaestus/write-must-fail 2>/dev/null; then
      exit 93
  fi
  printf 'state-ok\n' > /var/lib/hephaestus/golden-state
  test "$(cat /var/lib/hephaestus/golden-state)" = "state-ok"
  printf 'agent edit\n' > /workspace/work/input.txt
  printf 'durable report\n' > /workspace/work/reports/result.txt
  "#;

#[tokio::test(flavor = "multi_thread")]
#[serial]
#[allow(clippy::too_many_lines, clippy::large_stack_frames)]
async fn bearer_push_starts_run_through_production_bootstrap() {
    drop(
        tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
            )
            .with_test_writer()
            .try_init(),
    );
    let workload_phase_timing = workload_phase_timing_from_environment();
    let (Ok(parent_database_url), Ok(nats_url)) = (
        std::env::var("HEPHAESTUS_POSTGRES_TEST_URL"),
        std::env::var("HEPHAESTUS_NATS_TEST_URL"),
    ) else {
        assert!(
            !cooking::enabled()
                && env::var("HEPHAESTUS_APP_LIBKRUN_E2E").as_deref() != Ok("1")
                && env::var("HEPHAESTUS_APP_GATEWAY_CADDY_E2E").as_deref() != Ok("1"),
            "explicit E2E execution requires HEPHAESTUS_POSTGRES_TEST_URL and HEPHAESTUS_NATS_TEST_URL"
        );
        return;
    };
    // The parent census is an explicit standalone verification mode. Ordinary
    // golden runs may share the parent database with legitimate writers, so
    // they never assert a whole-parent invariant.
    let parent_before = (env::var("HEPHAESTUS_GOLDEN_ASSERT_PARENT_ISOLATION").as_deref()
        == Ok("1"))
    .then_some(async { product_outbox_census(&parent_database_url).await });
    let parent_before = match parent_before {
        Some(census) => Some(census.await),
        None => None,
    };
    let isolated_database = IsolatedGoldenDatabase::create(&parent_database_url).await;
    let database_url = isolated_database.target_url.clone();
    let libkrun_e2e = env::var("HEPHAESTUS_APP_LIBKRUN_E2E").as_deref() == Ok("1");
    let cooking_build_proof = env::var("HEPHAESTUS_APP_COOKING_BUILD_PROOF").as_deref() == Ok("1");
    let cooking_service_build_proof =
        env::var("HEPHAESTUS_APP_COOKING_SERVICE_BUILD_PROOF").as_deref() == Ok("1");
    let caddy_tls = env::var("HEPHAESTUS_CADDY_TEST_TLS").as_deref() == Ok("1");
    // Ordinary golden tests keep their short timeout. The real Cooking proof
    // uses the production build limit, with a small margin for the observer's
    // final state poll and cleanup.
    let build_timeout = if cooking_build_proof {
        Duration::from_secs(15 * 60)
    } else {
        Duration::from_secs(30)
    };
    let cooking_wait_timeout = if cooking_build_proof {
        build_timeout + Duration::from_secs(30)
    } else {
        Duration::from_secs(300)
    };
    let browser_e2e = env::var("HEPHAESTUS_COOKING_BROWSER_E2E").as_deref() == Ok("1");
    let installed_ui_fixture =
        env::var("HEPHAESTUS_COOKING_INSTALLED_UI_FIXTURE").as_deref() == Ok("1");
    let gateway_caddy_e2e = env::var("HEPHAESTUS_APP_GATEWAY_CADDY_E2E").as_deref() == Ok("1");
    let gateway_service_e2e = env::var("HEPHAESTUS_APP_GATEWAY_SERVICE_E2E").as_deref() == Ok("1");
    let gateway_service_external_e2e =
        env::var("HEPHAESTUS_APP_GATEWAY_SERVICE_EXTERNAL_E2E").as_deref() == Ok("1");
    let gateway_service_revocation_e2e =
        env::var("HEPHAESTUS_APP_GATEWAY_SERVICE_REVOCATION_E2E").as_deref() == Ok("1");
    let gateway_service_cutover_e2e =
        env::var("HEPHAESTUS_APP_GATEWAY_SERVICE_CUTOVER_E2E").as_deref() == Ok("1");
    let gateway_service_failed_candidate_e2e =
        env::var("HEPHAESTUS_APP_GATEWAY_SERVICE_FAILED_CANDIDATE_E2E").as_deref() == Ok("1");
    let gateway_service_candidate_capacity_e2e =
        env::var("HEPHAESTUS_APP_GATEWAY_SERVICE_CANDIDATE_CAPACITY_E2E").as_deref() == Ok("1");
    let gateway_service_rollback_e2e =
        env::var("HEPHAESTUS_APP_GATEWAY_SERVICE_ROLLBACK_E2E").as_deref() == Ok("1");
    let gateway_service_log_rpc_e2e =
        env::var("HEPHAESTUS_APP_GATEWAY_SERVICE_LOG_RPC_E2E").as_deref() == Ok("1");
    let gateway_service_log_guest_e2e =
        env::var("HEPHAESTUS_APP_GATEWAY_SERVICE_LOG_GUEST_E2E").as_deref() == Ok("1");
    assert!(
        !cooking::enabled() || gateway_caddy_e2e,
        "cooking requires the joined Caddy/libkrun fixture"
    );
    assert!(
        !gateway_caddy_e2e || libkrun_e2e,
        "the joined Caddy gateway proof requires the real libkrun backend"
    );
    assert!(
        !gateway_service_e2e || (gateway_caddy_e2e && libkrun_e2e),
        "the persistent service proof requires the joined Caddy/libkrun fixture"
    );
    assert!(
        !gateway_service_external_e2e || gateway_service_e2e,
        "the external persistent-service proof requires the service fixture"
    );
    assert!(
        !gateway_service_revocation_e2e || gateway_service_external_e2e,
        "the service revocation proof requires the external daemon fixture"
    );
    assert!(
        !gateway_service_revocation_e2e || (gateway_caddy_e2e && libkrun_e2e),
        "the service revocation proof requires the joined Caddy/libkrun fixture"
    );
    assert!(
        !gateway_service_cutover_e2e || gateway_service_external_e2e,
        "the persistent-service cutover proof requires the external daemon fixture"
    );
    assert!(
        !gateway_service_failed_candidate_e2e || gateway_service_external_e2e,
        "the failed-candidate proof requires the external daemon fixture"
    );
    assert!(
        !gateway_service_candidate_capacity_e2e || gateway_service_cutover_e2e,
        "the candidate-capacity proof requires the persistent-service cutover fixture"
    );
    assert!(
        !gateway_service_candidate_capacity_e2e || !gateway_service_failed_candidate_e2e,
        "the candidate-capacity proof cannot combine with failed-candidate mode"
    );
    assert!(
        !gateway_service_candidate_capacity_e2e || !gateway_service_rollback_e2e,
        "the candidate-capacity proof cannot combine with rollback"
    );
    assert!(
        !gateway_service_rollback_e2e || gateway_service_cutover_e2e,
        "the persistent-service rollback proof requires the cutover fixture"
    );
    assert!(
        !gateway_service_revocation_e2e
            || (!gateway_service_cutover_e2e
                && !gateway_service_rollback_e2e
                && !gateway_service_failed_candidate_e2e
                && !gateway_service_candidate_capacity_e2e
                && !gateway_service_log_rpc_e2e
                && !gateway_service_log_guest_e2e),
        "the service revocation proof requires the initial published service mode"
    );
    assert!(
        !gateway_service_log_rpc_e2e || gateway_service_e2e,
        "the service-log RPC proof requires the persistent-service fixture"
    );
    assert!(
        !gateway_service_log_guest_e2e || gateway_service_e2e,
        "the guest service-log proof requires the persistent-service fixture"
    );
    assert!(
        !gateway_service_log_guest_e2e || (gateway_caddy_e2e && libkrun_e2e),
        "the guest service-log proof requires the real Caddy/libkrun fixture"
    );
    assert!(
        !gateway_service_log_guest_e2e || !gateway_service_log_rpc_e2e,
        "the guest service-log proof cannot combine with seeded service-log RPC rows"
    );
    assert!(
        !cooking_service_build_proof || cooking_build_proof,
        "the Cooking service build proof requires HEPHAESTUS_APP_COOKING_BUILD_PROOF=1"
    );
    assert!(
        !cooking_service_build_proof || cooking::enabled(),
        "the Cooking service build proof requires HEPHAESTUS_APP_COOKING_E2E=1"
    );
    assert!(
        !caddy_tls || cooking_service_build_proof,
        "Caddy TLS mode is restricted to the published Cooking service proof"
    );
    assert!(
        !cooking_service_build_proof || (gateway_caddy_e2e && libkrun_e2e),
        "the Cooking service build proof requires the joined Caddy/libkrun fixture"
    );
    assert!(
        !installed_ui_fixture || cooking_service_build_proof,
        "the installed UI fixture requires the Cooking service build proof"
    );
    assert!(
        !installed_ui_fixture || caddy_tls,
        "the installed UI fixture requires Caddy TLS"
    );
    assert!(
        !cooking_service_build_proof
            || (!gateway_service_e2e
                && !gateway_service_external_e2e
                && !gateway_service_cutover_e2e
                && !gateway_service_failed_candidate_e2e
                && !gateway_service_candidate_capacity_e2e
                && !gateway_service_rollback_e2e
                && !gateway_service_revocation_e2e
                && !gateway_service_log_rpc_e2e
                && !gateway_service_log_guest_e2e),
        "the Cooking service build proof cannot combine with seeded or alternate service modes"
    );
    assert!(
        !gateway_service_log_guest_e2e || !gateway_service_external_e2e,
        "the guest service-log proof cannot use the external daemon early-return path"
    );
    assert!(
        !gateway_service_log_guest_e2e
            || (!gateway_service_cutover_e2e
                && !gateway_service_failed_candidate_e2e
                && !gateway_service_rollback_e2e),
        "the guest service-log proof requires the initial persistent service revision"
    );
    #[cfg(not(feature = "test-fixtures"))]
    assert!(
        !gateway_service_log_rpc_e2e,
        "the service-log RPC proof requires --features test-fixtures"
    );
    #[cfg(not(feature = "test-fixtures"))]
    assert!(
        !gateway_service_log_guest_e2e,
        "the guest service-log proof requires --features test-fixtures"
    );
    assert!(
        !gateway_service_e2e || !cooking_build_proof,
        "the persistent service proof is incompatible with the Cooking build proof"
    );
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .expect("connect golden PostgreSQL");
    Box::pin(async {
    let temporary = tempfile::tempdir().expect("golden temporary root");
    let root = temporary.path().canonicalize().expect("canonical root");
    let release_artifact_root = if gateway_service_external_e2e {
        root.join("artifacts").join("releases")
    } else {
        root.join("release-artifacts")
    };
    let repository_root = root.join("repositories");
    let storage = Arc::new(
        GitStorage::initialize(&repository_root)
            .await
            .expect("fixture Git storage"),
    );
    let fixture_repository = PgForgeRepository::new(pool.clone(), Arc::clone(&storage));
    let browser_oidc_issuer = golden_issuer();
    let user_id = UserId::new();
    let organization_id = OrganizationId::new();
    sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, 'Golden User')")
        .bind(user_id.as_uuid())
        .execute(&pool)
        .await
        .expect("seed user");
    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, $2)")
        .bind(organization_id.as_uuid())
        .bind(format!("golden-{organization_id}"))
        .execute(&pool)
        .await
        .expect("seed organization");
    sqlx::query(
        "INSERT INTO organization_members (organization_id, user_id, role)
           VALUES ($1, $2, 'owner')",
    )
    .bind(organization_id.as_uuid())
    .bind(user_id.as_uuid())
    .execute(&pool)
    .await
    .expect("seed organization owner");
    let outsider_id = UserId::new();
    sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, 'Cooking Outsider')")
        .bind(outsider_id.as_uuid())
        .execute(&pool)
        .await
        .expect("seed cooking outsider");
    sqlx::query(
        "INSERT INTO external_identities
           (user_id, issuer, subject, provider_metadata)
           VALUES ($1, $2, 'golden-subject', '{}')",
    )
    .bind(user_id.as_uuid())
    .bind(&browser_oidc_issuer)
    .execute(&pool)
    .await
    .expect("seed external identity");
    sqlx::query(
        "INSERT INTO external_identities
           (user_id, issuer, subject, provider_metadata)
           VALUES ($1, $2, 'outsider', '{}')",
    )
    .bind(outsider_id.as_uuid())
    .bind(&browser_oidc_issuer)
    .execute(&pool)
    .await
    .expect("seed cooking outsider identity");
    let owner_browser_session = seed_golden_browser_session(
        &pool,
        user_id,
        &browser_oidc_issuer,
        "golden-subject",
    )
    .await;
    #[cfg(feature = "test-fixtures")]
    let outsider_browser_session =
        seed_golden_browser_session(&pool, outsider_id, &browser_oidc_issuer, "outsider").await;
    let project = fixture_repository
        .create_project_trusted(organization_id, "golden-project")
        .await
        .expect("seed project");
    sqlx::query("INSERT INTO project_maintainers (project_id, user_id) VALUES ($1, $2)")
        .bind(project.id.as_uuid())
        .bind(user_id.as_uuid())
        .execute(&pool)
        .await
        .expect("seed project maintainer");
    // The config is resolved by the production Git-receive path, so seed the
    // same immutable catalog record that the runtime maps to the libkrun root
    // filesystem below. This keeps the composed proof on the real resolver
    // rather than retaining the previous, now-invalid ad-hoc image syntax.
    sqlx::query(
        "INSERT INTO oci_images
             (id, key, display_name, image_reference, toolchains, architectures,
              availability_state, provenance, platform_policy_version)
         VALUES ($1, 'golden-root', 'Golden root', $2, '[]'::jsonb,
                 ARRAY['x86_64'], 'available', '{}'::jsonb, 'golden/v1')",
    )
    .bind(uuid::Uuid::new_v4())
    .bind(ROOT_IMAGE)
    .execute(&pool)
    .await
    .expect("seed selected OCI image");
    let repository = fixture_repository
        .create_repository_trusted(&CreateRepository {
            project_id: project.id,
            name: String::from("golden-repository"),
            default_branch: GitRef::parse("refs/heads/main").expect("default ref"),
            is_public: false,
            agent_runs_enabled: true,
        })
        .await
        .expect("seed repository");
    let seeded_instance = seed_reusable_instance(
        &pool,
        user_id,
        project.id.as_uuid(),
        repository.id.as_uuid(),
        &release_artifact_root,
        libkrun_e2e,
        None,
        "golden-agent",
    )
    .await;
    let mut brokered_fixture = if libkrun_e2e && !cooking_build_proof {
        Some(if cooking::enabled() {
            cooking::seed_brokered_fixture(
                &pool,
                user_id,
                organization_id,
                project.id.as_uuid(),
                &seeded_instance,
            )
            .await
        } else {
            seed_brokered_https_fixture(
                &pool,
                user_id,
                organization_id,
                project.id.as_uuid(),
                &seeded_instance,
            )
            .await
        })
    } else {
        None
    };
    // This mailbox is created before the daemon starts so the released
    // gateway revision can be bound to it immutably.  The guest only sees the
    // symbolic `deliver` slot, never this UUID or the producer identity.
    let gateway_mailbox = if gateway_caddy_e2e && !cooking_build_proof {
        let mailbox_id = MailboxId::new();
        PostgresMailboxRepository::new(pool.clone())
            .ensure_mailbox(
                project.id.as_uuid(),
                mailbox_id,
                runtime_types::AgentInstanceId::from_uuid(seeded_instance.instance),
            )
            .await
            .expect("create bound gateway golden mailbox");
        Some(mailbox_id)
    } else {
        None
    };
    let gateway_service_fixture = if gateway_service_e2e && !cooking_build_proof {
        let service_agent =
            seed_gateway_service_release_agent(&pool, &seeded_instance, &release_artifact_root)
                .await;
        Some(
            seed_gateway_service_route(
                &pool,
                user_id,
                project.id.as_uuid(),
                repository.id.as_uuid(),
                seeded_instance.release,
                service_agent,
                gateway_service_log_guest_e2e,
            )
            .await,
        )
    } else {
        None
    };
    let gateway_edge = if gateway_caddy_e2e && !cooking_build_proof {
        let gateway_agent =
            seed_gateway_release_agent(&pool, &seeded_instance, &release_artifact_root).await;
        let fixture = brokered_fixture
            .as_ref()
            .expect("joined gateway proof has brokered fixture authority");
        let gateway_fixture = seed_gateway_brokered_route(
            &pool,
            user_id,
            project.id.as_uuid(),
            repository.id.as_uuid(),
            seeded_instance.release,
            gateway_agent,
            fixture.import_id,
            fixture.version_id,
            gateway_mailbox.expect("joined gateway mailbox"),
        )
        .await;
        let dispatcher = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("reserve gateway dispatcher listener");
        let dispatcher_listen = dispatcher
            .local_addr()
            .expect("gateway dispatcher listener address");
        drop(dispatcher);
        Some((
            GatewayEdgeConfig {
                caddy_admin_url: env::var("HEPHAESTUS_CADDY_TEST_ADMIN_URL")
                    .expect("joined Caddy admin URL"),
                caddy_configuration_template: caddy_configuration(
                    &env::var("HEPHAESTUS_CADDY_TEST_ADMIN_URL").expect("joined Caddy admin URL"),
                ),
                caddy_server_name: String::from("shared"),
                dispatcher_listen,
                public_authority: String::from("gateway.golden.invalid"),
                ui_origin: None,
            },
            gateway_fixture,
        ))
    } else {
        None
    };
    // The installed UI fixture publishes an HTTP service gateway before the
    // later Cooking-service restart. Give the initial daemon its real gateway
    // supervisor so that desired service revisions can pass readiness and
    // become active before InstallUi resolves the managed route. The UI origin
    // is intentionally added only by the later restart below.
    let initial_gateway_edge = if installed_ui_fixture {
        let dispatcher = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("reserve installed UI gateway dispatcher listener");
        let dispatcher_listen = dispatcher
            .local_addr()
            .expect("installed UI gateway dispatcher listener address");
        drop(dispatcher);
        Some(GatewayEdgeConfig {
            caddy_admin_url: env::var("HEPHAESTUS_CADDY_TEST_ADMIN_URL")
                .expect("joined Caddy admin URL"),
            caddy_configuration_template: caddy_configuration(
                &env::var("HEPHAESTUS_CADDY_TEST_ADMIN_URL").expect("joined Caddy admin URL"),
            ),
            caddy_server_name: String::from("shared"),
            dispatcher_listen,
            public_authority: String::from("gateway.golden.invalid"),
            ui_origin: None,
        })
    } else {
        gateway_edge.as_ref().map(|(config, _)| config.clone())
    };

    let mut backend_fixture = backend_fixture(&root).await;
    let root_image = backend_fixture.root_image.clone();
    let mut root_images = BTreeMap::from([(
        String::from(ROOT_IMAGE),
        RootFilesystem::Directory {
            host_path: root_image.clone(),
        },
    )]);
    if cooking_build_proof {
        let python_image =
            env::var("HEPHAESTUS_LIBKRUN_UBUNTU_IMAGE").expect("cooking Python image reference");
        let rust_image = env::var("HEPHAESTUS_LIBKRUN_RUST_BUILDER_IMAGE")
            .expect("cooking Rust builder image reference");
        for (key, reference) in [
            ("python-ubuntu", &python_image),
            ("rust-ubuntu", &rust_image),
        ] {
            sqlx::query(
                "INSERT INTO oci_images
                   (id, key, display_name, image_reference, toolchains, architectures,
                    availability_state, provenance, platform_policy_version)
                 VALUES ($1, $2, $3, $4, '[]'::jsonb, ARRAY['x86_64'],
                         'available', '{}'::jsonb, 'cooking-build-proof/v1')",
            )
            .bind(uuid::Uuid::new_v4())
            .bind(key)
            .bind(key)
            .bind(reference)
            .execute(&pool)
            .await
            .expect("seed cooking build-proof image catalog entry");
        }
        root_images.insert(
            python_image,
            RootFilesystem::Directory {
                host_path: PathBuf::from(
                    env::var("HEPHAESTUS_LIBKRUN_ROOTFS").expect("cooking Python root filesystem"),
                ),
            },
        );
        root_images.insert(
            rust_image,
            RootFilesystem::Directory {
                host_path: PathBuf::from(
                    env::var("HEPHAESTUS_LIBKRUN_RUST_BUILDER_ROOT")
                        .expect("cooking Rust builder root filesystem"),
                ),
            },
        );
    }
    let cooking_worker = if cooking_build_proof {
        let worker = cooking_oci_worker_config(&root, &repository_root, workload_phase_timing)
            .expect("cooking OCI worker environment");
        root_images.insert(
            worker.builder_vm_image.to_string(),
            RootFilesystem::Directory {
                host_path: PathBuf::from(
                    env::var("HEPHAESTUS_TEST_OCI_BUILDER_ROOT").expect("cooking OCI builder root"),
                ),
            },
        );
        root_images.insert(
            worker.verifier_vm_image.to_string(),
            RootFilesystem::Directory {
                host_path: PathBuf::from(
                    env::var("HEPHAESTUS_TEST_OCI_VERIFIER_ROOT")
                        .expect("cooking OCI verifier root"),
                ),
            },
        );
        Some(worker)
    } else {
        None
    };
    let cooking_layout_mount_roots = cooking_worker
        .as_ref()
        .map(|worker| cooking_base_layout_mount_roots(&worker.runtime.image_layouts))
        .transpose()
        .unwrap_or_else(|(path, error)| {
            panic!(
                "canonicalize configured cooking OCI base layout {}: {error}",
                path.display()
            )
        })
        .unwrap_or_default();
    if let VmBackendConfig::Libkrun(provider) = &mut backend_fixture.backend {
        // The OCI job mounts only the reviewed, operator-owned base layouts
        // supplied by its workflow manifest. Keep each configured layout an
        // explicit provider allowlist root instead of guessing a source-tree
        // cache path or broadening the fixture to the entire local root.
        provider.mount_roots.extend(cooking_layout_mount_roots);
        if cooking_build_proof {
            // The one-shot OCI builder formats its private scratch disk under
            // the fixture root; it must be a disk allowlist root as well as a
            // worker filesystem root. Ordinary libkrun golden runs do not
            // create this optional cooking directory.
            provider
                .disk_roots
                .push(root.join("repository-images/scratch"));
        }
    }
    let secret_broker_socket = root.join("secret-broker.sock");
    let observer = if cooking_build_proof {
        assert!(
            libkrun_e2e,
            "cooking build proof requires the real libkrun backend"
        );
        let required_image_roots = cooking_worker
            .as_ref()
            .map(|worker| vec![worker.rootfs_root.clone()])
            .unwrap_or_default();
        Some(
            backend_fixture
                .install_vm_observer(
                    cooking_confinement::credential_patterns(),
                    &secret_broker_socket,
                    &required_image_roots,
                )
                .expect("install real-provider VM specification observer"),
        )
    } else {
        None
    };
    let mut transient_runtime_roots = backend_fixture.transient_runtime_roots;
    transient_runtime_roots.push(root.join("workspaces"));
    let backend = git_backend().await;
    let git_pre_receive_hook = root.join("git-hooks/pre-receive");
    std::fs::create_dir_all(
        git_pre_receive_hook
            .parent()
            .expect("Git hook has a parent directory"),
    )
    .expect("Git hook directory");
    std::fs::write(&git_pre_receive_hook, b"#!/bin/sh\nexit 1\n")
        .expect("write fail-closed Git hook fixture");
    std::fs::set_permissions(
        &git_pre_receive_hook,
        std::os::unix::fs::PermissionsExt::from_mode(0o700),
    )
    .expect("Git hook fixture mode");
    let secret_mount_root = root.join("secret-mounts");
    std::fs::create_dir(&secret_mount_root).expect("secret mount root");
    std::fs::set_permissions(
        &secret_mount_root,
        std::os::unix::fs::PermissionsExt::from_mode(0o700),
    )
    .expect("secret mount root mode");
    let mut app_config = AppConfig {
        database_url: database_url.clone(),
        nats_url: nats_url.clone(),
        http_listen: "127.0.0.1:0".parse().expect("ephemeral listen address"),
        rpc_mediator_signing_key: hephaestus_app::rpc::mediator_signing_key(
            b"golden-internal-command-token-with-sufficient-entropy",
        ),
        repository_root: repository_root.clone(),
        git_http_backend: backend,
        git_pre_receive_hook,
        git_http_limits: git_http::GitHttpLimits::default(),
        oidc: OidcConfig {
            issuer: browser_oidc_issuer.clone(),
            audience: String::from(AUDIENCE),
            algorithm: Algorithm::HS256,
            decoding_key: jsonwebtoken::DecodingKey::from_secret(SIGNING_SECRET),
        },
        registry: golden_registry_config(),
        gateway_edge: initial_gateway_edge,
        volumes: LocalVolumeConfig {
            volume_root: backend_fixture.volume_root,
            transient_runtime_roots,
            host_id: String::from("golden-host"),
            lease_duration: Duration::from_secs(30),
            mkfs_ext4: mkfs_ext4(),
        },
        workspaces: LocalWorkspaceConfig {
            workspace_root: root.join("workspaces"),
            artifact_root: root.join("artifacts"),
            repository_root: repository_root.clone(),
            git_binary: git_binary().await,
            limits: WorkspaceLimits::default(),
        },
        run_runtime: LocalRunRuntimeConfig {
            runtime_root: root.join("run-runtime"),
            release_artifact_root: root.join("release-artifacts"),
        },
        runtime_authority_handoff_root: root.join("runtime-authority-handoffs"),
        runtime_authority_handoff_key: [0x39; 32],
        runtime_authority_session_ttl: Duration::from_secs(3_600),
        build_workspace_root: root.join("isolated-builds"),
        build_timeout,
        secret_mounts: EphemeralSecretConfig {
            root: secret_mount_root,
            require_memory_filesystem: false,
        },
        secret_keys: LocalKeyProvider::new("golden/v1", [("golden/v1", [17_u8; 32])])
            .expect("secret key"),
        secret_broker_socket,
        secret_broker_adapter: brokered_fixture.as_ref().map_or_else(
            || Arc::new(DenyingBrokerAdapter) as Arc<dyn secret_application::BrokerAdapter>,
            |fixture| fixture.upstream.adapter(),
        ),
        vm_backend: backend_fixture.backend,
        root_images,
        oci_builder: cooking_worker,
        runtime_policy: hephaestus_app::RuntimePolicy {
            version: String::from("golden/v1"),
            max_vcpus: 2,
            max_memory_mib: 1_024,
            allow_broker_only: true,
            allow_egress: true,
        },
        agent_state_capacity_bytes: 16 * 1024 * 1024,
        worker_concurrency: 4,
        outbox_poll_interval: Duration::from_millis(10),
        outbox_batch_size: 20,
        startup_timeout: Duration::from_secs(10),
        shutdown_timeout: Duration::from_secs(10),
    };
    if gateway_service_external_e2e {
        let service_fixture = gateway_service_fixture
            .as_ref()
            .expect("external persistent-service fixture");
        let (gateway_config, _) = gateway_edge
            .as_ref()
            .expect("external persistent-service gateway configuration");
        exercise_external_gateway_service_warm_path(
            &pool,
            service_fixture,
            gateway_config,
            &app_config,
            &root,
            &root_image,
            &release_artifact_root,
            &AuthenticatedIdentity::new(
                user_id,
                &browser_oidc_issuer,
                "golden-subject",
                serde_json::json!({}),
                RequestId::new(),
            ),
        )
        .await;
        cleanup_streams(&nats_url).await;
        return;
    }
    let app = HephaestusApp::build(app_config.clone())
        .await
        .expect("build production application");
    let daemon_readiness_timer =
        WorkloadPhaseTimer::start("gateway-readiness", workload_phase_timing);
    let running = async move { app.start().await }.await;
    daemon_readiness_timer.finish(running.is_ok());
    let running = running.expect("start ready application");
    // The Cooking proof deliberately spans several production builds before
    // it creates the separate blog repository. Keep its fixture assertion
    // valid for the bounded 45-minute host trial while ordinary golden tests
    // retain the shorter token lifetime.
    let token = signed_token(if cooking_build_proof {
        Duration::from_secs(45 * 60)
    } else {
        Duration::from_secs(5 * 60)
    });
    if cooking_build_proof {
        let installed_ui_fixture =
            env::var("HEPHAESTUS_COOKING_INSTALLED_UI_FIXTURE").as_deref() == Ok("1");
        if installed_ui_fixture {
            assert!(
                browser_e2e,
                "installed UI fixture requires the browser E2E phase to be enabled"
            );
        }
        let source_root =
            PathBuf::from(env::var("HEPHAESTUS_COOKING_SOURCE_ROOT").expect("cooking source root"));
        let identity = AuthenticatedIdentity::new(
            user_id,
            &browser_oidc_issuer,
            "golden-subject",
            serde_json::json!({}),
            RequestId::new(),
        );
        let rpc_token = |audience: &str| {
            let now = OffsetDateTime::now_utc().unix_timestamp();
            encode(
                &Header::new(Algorithm::HS256),
                &serde_json::json!({
                    "iss": "hephaestus-web-mediator",
                    "sub": user_id.to_string(),
                    "aud": audience,
                    "iat": now,
                    "nbf": now,
                    "exp": now + 25,
                    "jti": uuid::Uuid::new_v4().to_string(),
                    "sid": owner_browser_session.to_protocol_string()
                }),
                &EncodingKey::from_secret(&hephaestus_app::rpc::mediator_signing_key(
                    b"golden-internal-command-token-with-sufficient-entropy",
                )),
            )
            .expect("sign cooking build-proof mediator token")
        };
        if env::var("HEPHAESTUS_COOKING_OCI_BASE_IMPORT_DIAGNOSTIC").as_deref() == Ok("1") {
            let context = cooking_builds::CookingBuildContext {
                pool: &pool,
                running: &running,
                root: &root,
                source_root: &source_root,
                project_id: project.id,
                repositories: &fixture_repository,
                identity: cooking_builds::CookingIdentity {
                    actor: &identity,
                    git_token: &token,
                    rpc_token: &rpc_token,
                },
                timeout: cooking_wait_timeout,
            };
            cooking_builds::create_cooking_blog_repository(&context)
                .await
                .expect("targeted cooking OCI base-import diagnostic");
            running
                .shutdown()
                .await
                .expect("targeted OCI diagnostic shutdown");
            cleanup_streams(&nats_url).await;
            return;
        }
        let retry_fixture =
            if browser_e2e || env::var("HEPHAESTUS_COOKING_UPDATE_E2E").as_deref() == Ok("1") {
                let retry_repository = fixture_repository
                    .create_repository_trusted(&CreateRepository {
                        project_id: project.id,
                        name: format!("golden-retry-source-{}", uuid::Uuid::new_v4()),
                        default_branch: GitRef::parse("refs/heads/main").expect("default ref"),
                        is_public: false,
                        agent_runs_enabled: true,
                    })
                    .await
                    .expect("seed browser retry source repository");
                let retry_instance = seed_reusable_instance(
                    &pool,
                    user_id,
                    project.id.as_uuid(),
                    retry_repository.id.as_uuid(),
                    &root.join("release-artifacts"),
                    libkrun_e2e,
                    Some(GOLDEN_AGENT.as_bytes()),
                    "golden-retry-source",
                )
                .await;
                let source = create_forge_source_run(
                    &pool,
                    &running,
                    &root,
                    retry_repository.id.as_uuid(),
                    retry_instance.instance,
                    &token,
                    libkrun_e2e,
                    true,
                    ForgeSourceContent::GoldenAgent,
                )
                .await;
                Some(ForgeRetryFixture {
                    repository_id: retry_repository.id.as_uuid(),
                    instance: retry_instance,
                    source_run_id: source.run_id,
                })
            } else {
                None
            };
        let cooking_context = cooking_builds::CookingBuildContext {
            pool: &pool,
            running: &running,
            root: &root,
            source_root: &source_root,
            project_id: project.id,
            repositories: &fixture_repository,
            identity: cooking_builds::CookingIdentity {
                actor: &identity,
                git_token: &token,
                rpc_token: &rpc_token,
            },
            timeout: cooking_wait_timeout,
        };
        let installed_reference_uis = if installed_ui_fixture {
            Some(
                cooking_builds::build_and_install_reference_uis(&cooking_context, organization_id)
                    .await
                    .expect("build and install reference UIs through production boundaries"),
            )
        } else {
            None
        };
        if cooking_service_build_proof {
            let published = cooking_service_build::build_and_publish_cooking_service(
                &cooking_context,
            )
            .await
            .expect("real cooking-service source build and publish proof");
            assert_eq!(published.actor_id, user_id);
            assert!(!published.repository_id.as_uuid().is_nil());
            assert!(!published.source_commit.is_empty());
            assert!(!published.build_request_id.is_nil());
            assert!(!published.release_id.is_nil());
            assert!(!published.release_agent_id.is_nil());
            assert!(!published.version.is_empty());
            assert!(published.source_path.is_dir());
            assert!(published.working_path.is_dir());
            cooking_builds::wait_for_cooking_build_quiescence(
                &pool,
                project.id,
                "golden-cooking-oci-materialization",
                cooking_wait_timeout,
            )
            .await;
            let configured = cooking_service_build::install_and_configure_cooking_service(
                &cooking_context,
                &published,
            )
            .await
            .expect("install and configure published cooking service");
            let network: String = sqlx::query_scalar(
                "SELECT release_agent.runtime_contract #>> '{policy_ceiling,network}'
                   FROM gateway_revisions revision
                   JOIN release_agents release_agent
                     ON release_agent.id = revision.release_agent_id
                  WHERE revision.id = $1",
            )
            .bind(configured.revision_id)
            .fetch_one(&pool)
            .await
            .expect("read published cooking service network policy");
            assert_eq!(network, "disabled", "cooking service guest network policy");

            let dispatcher = TcpListener::bind("127.0.0.1:0")
                .await
                .expect("reserve cooking service dispatcher listener");
            let dispatcher_listen = dispatcher
                .local_addr()
                .expect("cooking service dispatcher listener address");
            drop(dispatcher);
            app_config.gateway_edge = Some(GatewayEdgeConfig {
                caddy_admin_url: env::var("HEPHAESTUS_CADDY_TEST_ADMIN_URL")
                    .expect("joined Caddy admin URL"),
                caddy_configuration_template: caddy_configuration(
                    &env::var("HEPHAESTUS_CADDY_TEST_ADMIN_URL").expect("joined Caddy admin URL"),
                ),
                caddy_server_name: String::from("shared"),
                dispatcher_listen,
                public_authority: String::from("gateway.golden.invalid"),
                ui_origin: installed_reference_uis.as_ref().map(|_| {
                    let listener = reserve_installed_ui_listener();
                    installed_ui_origin_config(listener)
                }),
            });
            running
                .shutdown()
                .await
                .expect("pre-Caddy cooking service daemon shutdown");
            let running = Box::pin(restart_application(app_config.clone())).await;
            let service_fixture = GatewayServiceGoldenFixture {
                gateway_id: configured.gateway_id,
                revision_id: configured.revision_id,
            };
            let service_instance_id =
                wait_for_gateway_service_ready(&pool, &service_fixture).await;
            let pointers: (Option<uuid::Uuid>, Option<uuid::Uuid>) = sqlx::query_as(
                "SELECT active_revision_id, desired_service_revision_id
                   FROM gateways
                  WHERE id = $1",
            )
            .bind(service_fixture.gateway_id)
            .fetch_one(&pool)
            .await
            .expect("read cooking service gateway revision pointers");
            assert_eq!(pointers, (Some(service_fixture.revision_id), Some(service_fixture.revision_id)));
            let admin_url =
                env::var("HEPHAESTUS_CADDY_TEST_ADMIN_URL").expect("joined Caddy admin URL");
            let applied_config = wait_for_caddy_configuration(&admin_url, "/gateway/service").await;
            assert!(
                applied_config.contains("/gateway/service"),
                "Caddy must contain the published cooking service route: {applied_config}"
            );
            let public_url =
                env::var("HEPHAESTUS_CADDY_TEST_PUBLIC_URL").expect("joined Caddy public URL");
            let client = published_cooking_service_client();
            let service_body = client
                .get(format!("{public_url}/gateway/service"))
                .send()
                .await
                .expect("public cooking service request")
                .error_for_status()
                .expect("public cooking service request succeeds")
                .bytes()
                .await
                .expect("read public cooking service response");
            assert_eq!(service_body.as_ref(), b"cooking service");
            let identity_before = exercise_published_cooking_service_identity(&public_url).await;
            exercise_published_cooking_service_isolation(&public_url, &admin_url).await;
            exercise_published_cooking_service_metadata(&public_url).await;
            // The isolation probe opens a guest-loopback request. Comparing
            // identity on both sides proves it did not replace the process.
            let identity_after = exercise_published_cooking_service_identity(&public_url).await;
            assert_eq!(identity_before.pid, identity_after.pid);
            assert_eq!(identity_before.startup_id, identity_after.startup_id);
            if caddy_tls {
                println!("REAL_COOKING_SERVICE_HTTPS_METADATA=1");
            }
            println!(
                "REAL_COOKING_SERVICE_ISOLATION=1 pid={} startup_id={}",
                identity_after.pid, identity_after.startup_id
            );
            let identity_proof = identity_after;
            if let Some(installed_uis) = installed_reference_uis {
                run_installed_ui_browser_phase(InstalledUiBrowserContext {
                    pool: &pool,
                    running: &running,
                    database_url: &database_url,
                    organization_id,
                    installed_uis,
                    actor_id: user_id.as_uuid(),
                    workload_phase_timing,
                })
                .await;
            }
            let resource_paths = {
                let vm_id = format!("gateway-service-{service_instance_id}");
                let provider_runtime_root = PathBuf::from(
                    env::var("HEPHAESTUS_LIBKRUN_RUNTIME_ROOT")
                        .expect("libkrun runtime root for Cooking service proof"),
                );
                let cgroup_root = PathBuf::from(
                    env::var("HEPHAESTUS_LIBKRUN_CGROUP_ROOT")
                        .expect("libkrun cgroup root for Cooking service proof"),
                );
                (
                    provider_runtime_root.join(&vm_id),
                    cgroup_root.join(&vm_id),
                    root.join("run-runtime")
                        .join("gateway-services")
                        .join(service_instance_id.to_string()),
                )
            };
            let (provider_runtime, cgroup, materializer) = &resource_paths;
            assert!(provider_runtime.is_dir());
            assert!(cgroup.is_dir());
            assert!(materializer.is_dir());
            running
                .shutdown()
                .await
                .expect("cooking service daemon shutdown");
            let state: Option<String> = sqlx::query_scalar(
                "SELECT state FROM gateway_service_instances
                  WHERE id = $1 AND gateway_id = $2 AND revision_id = $3",
            )
            .bind(service_instance_id)
            .bind(service_fixture.gateway_id)
            .bind(service_fixture.revision_id)
            .fetch_optional(&pool)
            .await
            .expect("read cleaned cooking service instance");
            assert_eq!(state.as_deref(), Some("cleaned"));
            assert!(!provider_runtime.exists());
            assert!(!cgroup.exists());
            assert!(!materializer.exists());
            cleanup_streams(&nats_url).await;
            println!(
                "REAL_COOKING_SERVICE_BUILD_PROOF=1 gateway={} revision={} instance={} pid={} startup_id={}",
                service_fixture.gateway_id,
                service_fixture.revision_id,
                service_instance_id,
                identity_proof.pid,
                identity_proof.startup_id
            );
            return;
        }
        // These repositories share only their project: Python/Rust release builds
        // do not consume the blog's Hugo image. Keep same-family release mutations
        // serial while the independent OCI build and verification make progress.
        let (prepared_releases, blog_repository) = Box::pin(join_cooking_preparation(
            async {
                let builds = {
                    let production_project_build_timer = WorkloadPhaseTimer::start(
                        "production-project-build",
                        workload_phase_timing,
                    );
                    let result =
                        cooking_builds::build_and_publish(cooking_builds::CookingBuildContext {
                            pool: &pool,
                            running: &running,
                            root: &root,
                            source_root: &source_root,
                            project_id: project.id,
                            repositories: &fixture_repository,
                            identity: cooking_builds::CookingIdentity {
                                actor: &identity,
                                git_token: &token,
                                rpc_token: &rpc_token,
                            },
                            timeout: cooking_wait_timeout,
                        })
                        .await;
                    production_project_build_timer.finish(result.is_ok());
                    result.expect("real cooking source build and publish proof")
                };
                let adversarial_agent_build = cooking_builds::build_and_publish_adversarial_agent(
                    &cooking_builds::CookingBuildContext {
                        pool: &pool,
                        running: &running,
                        root: &root,
                        source_root: &source_root,
                        project_id: project.id,
                        repositories: &fixture_repository,
                        identity: cooking_builds::CookingIdentity {
                            actor: &identity,
                            git_token: &token,
                            rpc_token: &rpc_token,
                        },
                        timeout: cooking_wait_timeout,
                    },
                    &builds.agent,
                )
                .await
                .expect("publish adversarial cooking agent destination release");
                assert_ne!(
                    adversarial_agent_build.release_id, builds.agent.release_id,
                    "the adversarial agent must use a distinct published release"
                );
                let adversarial_gateway_build =
                    cooking_builds::build_and_publish_adversarial_gateway(
                        &cooking_builds::CookingBuildContext {
                            pool: &pool,
                            running: &running,
                            root: &root,
                            source_root: &source_root,
                            project_id: project.id,
                            repositories: &fixture_repository,
                            identity: cooking_builds::CookingIdentity {
                                actor: &identity,
                                git_token: &token,
                                rpc_token: &rpc_token,
                            },
                            timeout: cooking_wait_timeout,
                        },
                        &builds.gateway,
                    )
                    .await
                    .expect("publish adversarial foreign-slot gateway release");
                assert_eq!(
                    adversarial_gateway_build.repository_id, builds.gateway.repository_id,
                    "the adversarial release must remain in the canonical release family"
                );
                assert_ne!(
                    adversarial_gateway_build.release_id, builds.gateway.release_id,
                    "the adversarial probe must use a distinct published release"
                );
                for published in [&builds.gateway, &builds.agent, &adversarial_agent_build] {
                    assert_eq!(published.actor_id, user_id);
                    assert!(!published.repository_id.as_uuid().is_nil());
                    assert!(!published.source_commit.is_empty());
                    assert!(!published.build_request_id.is_nil());
                    assert!(!published.release_id.is_nil());
                    assert!(!published.release_agent_id.is_nil());
                    assert!(!published.version.is_empty());
                    assert!(!published.build_definition_hash.is_empty());
                    assert!(!published.configuration_hash.is_empty());
                    assert!(!published.manifest_hash.is_empty());
                    assert!(published.source_path.is_dir());
                    assert!(published.working_path.is_dir());
                }
                let update_builds =
                    if env::var("HEPHAESTUS_COOKING_UPDATE_E2E").as_deref() == Ok("1") {
                        Some(
                            cooking_builds::build_and_publish_update_variants(
                                cooking_builds::CookingBuildContext {
                                    pool: &pool,
                                    running: &running,
                                    root: &root,
                                    source_root: &source_root,
                                    project_id: project.id,
                                    repositories: &fixture_repository,
                                    identity: cooking_builds::CookingIdentity {
                                        actor: &identity,
                                        git_token: &token,
                                        rpc_token: &rpc_token,
                                    },
                                    timeout: cooking_wait_timeout,
                                },
                                &builds.agent,
                            )
                            .await
                            .expect("publish cooking update-hook variants"),
                        )
                    } else {
                        None
                    };
                (
                    builds,
                    adversarial_agent_build,
                    adversarial_gateway_build,
                    update_builds,
                )
            },
            async {
                let blog_repository = cooking_builds::create_cooking_blog_repository(
                    &cooking_builds::CookingBuildContext {
                        pool: &pool,
                        running: &running,
                        root: &root,
                        source_root: &source_root,
                        project_id: project.id,
                        repositories: &fixture_repository,
                        identity: cooking_builds::CookingIdentity {
                            actor: &identity,
                            git_token: &token,
                            rpc_token: &rpc_token,
                        },
                        timeout: cooking_wait_timeout,
                    },
                )
                .await
                .expect("create and push separate cooking blog repository");
                assert!(!blog_repository.source_commit.is_empty());
                blog_repository
            },
        ))
        .await;
        let (builds, adversarial_agent_build, adversarial_gateway_build, update_builds) =
            prepared_releases;
        cooking_builds::wait_for_cooking_build_quiescence(
            &pool,
            project.id,
            "golden-cooking-oci-materialization",
            cooking_wait_timeout,
        )
        .await;
        let instance = cooking_builds::prepare_cooking_instance(
            &cooking_context,
            builds.agent.release_agent_id,
            blog_repository.repository_id,
            cooking_builds::cooking_agent_parameters(),
        )
        .await
        .expect("ImportAgent/CreateAttachment/CreateMailbox cooking instance");
        let actual_instance = SeededInstance {
            instance: instance.instance_id,
            revision: instance.revision_id,
            attachment: instance.attachment_id,
            release: builds.agent.release_id,
            release_agent: builds.agent.release_agent_id,
        };
        let actual_brokered = cooking::seed_brokered_fixture(
            &pool,
            user_id,
            organization_id,
            project.id.as_uuid(),
            &actual_instance,
        )
        .await;
        let cooking_update_rule_ids = update_builds.as_ref().map(|_| {
            (
                cooking_updates::BrokeredRuleIds::fresh(),
                cooking_updates::BrokeredRuleIds::fresh(),
                cooking_updates::BrokeredRuleIds::fresh(),
                cooking_updates::BrokeredRuleIds::fresh(),
            )
        });
        if let Some((migration, rollback, abnormal, browser)) = cooking_update_rule_ids {
            actual_brokered.upstream.register_rule_copies(&[
                (cooking::MODEL_RULE, migration.model),
                (cooking::RELAY_RULE, migration.relay),
                (migration.model, rollback.model),
                (migration.relay, rollback.relay),
                (migration.model, abnormal.model),
                (migration.relay, abnormal.relay),
                (migration.model, browser.model),
                (migration.relay, browser.relay),
            ]);
        }
        let cooking_inbound_placeholder =
            cooking_builds::cooking_inbound_placeholder(actual_brokered.version_id);
        let installed_gateway = cooking_builds::install_cooking_gateway(
            &cooking_context,
            builds.gateway.release_id,
            builds.gateway.repository_id,
        )
        .await
        .expect("install released cooking gateway");
        // The browser owns the first configuration when it is enabled. This
        // keeps its immutable-revision proof independent from the ordinary
        // RPC configure/replay proof used by the backend-only path.
        let mut actual_grant_id = None;
        if browser_e2e {
            assert_eq!(
                installed_gateway.revision_id,
                sqlx::query_scalar::<_, uuid::Uuid>(
                    "SELECT active_revision_id FROM gateways WHERE id = $1",
                )
                .bind(installed_gateway.gateway_id)
                .fetch_one(&pool)
                .await
                .expect("installed cooking gateway active revision"),
                "browser must start from the installed, unconfigured gateway revision"
            );
            let initial_binding_count: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM gateway_mailbox_bindings WHERE gateway_revision_id = $1",
            )
            .bind(installed_gateway.revision_id)
            .fetch_one(&pool)
            .await
            .expect("installed cooking gateway initial bindings");
            assert_eq!(
                initial_binding_count, 0,
                "browser must start from a gateway revision without mailbox bindings"
            );
        } else {
            let configured_revision = cooking_builds::configure_cooking_gateway(
                &cooking_context,
                installed_gateway,
                cooking_builds::cooking_gateway_parameters(
                    &cooking_inbound_placeholder,
                    1001,
                    1002,
                ),
                actual_brokered.import_id,
                actual_brokered.version_id,
                instance.mailbox_id,
            )
            .await
            .expect("ConfigureGateway/CreateMailboxBinding cooking gateway");
            assert_ne!(
                configured_revision.revision_id,
                installed_gateway.revision_id
            );
            assert!(!configured_revision.grant_id.is_nil());
            actual_grant_id = Some(configured_revision.grant_id);
        }
        assert_ne!(instance.instance_id, uuid::Uuid::nil());
        // Provision a real second mailbox through the instance RPC and point
        // a temporary immutable gateway revision at it. The released source
        // then asks the host to publish through an undeclared slot; the edge
        // must reject it before creating a foreign event or run.
        let foreign_instance = cooking_builds::prepare_cooking_instance_variant(
            &cooking_context,
            builds.agent.release_agent_id,
            blog_repository.repository_id,
            cooking_builds::cooking_agent_parameters(),
            "cooking-agent-foreign",
            "cooking-agent-foreign",
        )
        .await
        .expect("provision adversarial foreign mailbox");
        assert_ne!(foreign_instance.instance_id, instance.instance_id);
        assert_ne!(foreign_instance.mailbox_id, instance.mailbox_id);
        let fixture_path = if browser_e2e {
            Some(
                env::var("HEPHAESTUS_COOKING_BROWSER_FIXTURE_OUTPUT")
                    .expect("cooking browser fixture output path"),
            )
        } else {
            env::var("HEPHAESTUS_COOKING_BROWSER_FIXTURE_OUTPUT").ok()
        };
        if let Some(path) = fixture_path {
            let fixture = serde_json::json!({
                "organization_id": organization_id,
                "project_id": project.id,
                "repository_id": builds.gateway.repository_id,
                "release_id": builds.gateway.release_id,
                "release_agent_id": builds.agent.release_agent_id,
                "instance_id": instance.instance_id,
                "mailbox_id": instance.mailbox_id,
                "gateway_id": installed_gateway.gateway_id,
                "inbound_import_id": actual_brokered.import_id,
                "inbound_secret_version_id": actual_brokered.version_id,
                "inbound_selection": format!(
                    "{}|{}|/cooking/telegram|x-telegram-bot-api-secret-token",
                    actual_brokered.import_id, actual_brokered.version_id
                ),
                "parameters": {
                    "inbound_placeholder": cooking_inbound_placeholder.clone(),
                    "alice_provider_id": 1001,
                    "bob_provider_id": 1002
                },
                "installed_reference_uis": installed_reference_uis.map(|uis| serde_json::json!({
                    "organization_id": uis.organization_id,
                    "project_id": uis.project_id,
                    "static_installation_id": uis.static_ui.installation_id,
                    "static_generation_id": uis.static_ui.generation_id,
                    "managed_installation_id": uis.managed_ui.installation_id,
                    "managed_generation_id": uis.managed_ui.generation_id
                }))
            });
            tokio::fs::write(
                &path,
                serde_json::to_vec_pretty(&fixture).expect("cooking browser fixture JSON"),
            )
            .await
            .expect("write cooking browser fixture JSON");
            if browser_e2e {
                let issuer = env::var("HEPHAESTUS_COOKING_BROWSER_OIDC_ISSUER")
                    .expect("cooking browser OIDC issuer");
                let installed_browser = installed_reference_uis.is_some();
                let script_name = if installed_browser {
                    "../../scripts/run-installed-ui-e2e.sh"
                } else {
                    "../../scripts/run-ui-e2e-external.sh"
                };
                let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(script_name);
                let browser_timer =
                    WorkloadPhaseTimer::start("browser-initial", workload_phase_timing);
                let mut browser_command = tokio::process::Command::new(script);
                browser_command
                    .env("HEPHAESTUS_E2E_COOKING_FIXTURE", &path)
                    .env("HEPHAESTUS_E2E_EXTERNAL_DATABASE_URL", &database_url)
                    .env(
                        "HEPHAESTUS_E2E_EXTERNAL_RPC_ENDPOINT",
                        running.http_addr().to_string(),
                    )
                    .env(
                        "HEPHAESTUS_E2E_EXTERNAL_RPC_SECRET",
                        "golden-internal-command-token-with-sufficient-entropy",
                    )
                    .env("HEPHAESTUS_E2E_EXTERNAL_OIDC_ISSUER", issuer)
                    .env("HEPHAESTUS_E2E_COOKING_PHASE", "initial");
                if installed_browser {
                    browser_command
                        .env("HEPHAESTUS_PLATFORM_HTTPS_ORIGIN", installed_ui_platform_origin())
                        .env("HEPHAESTUS_UI_NAMESPACE", installed_ui_namespace())
                        .env(
                            "HEPHAESTUS_CADDY_TEST_CA_CERT",
                            env::var("HEPHAESTUS_CADDY_TEST_CA_CERT")
                                .expect("joined Caddy CA certificate for installed UI"),
                        );
                }
                let status = browser_command.status().await;
                browser_timer.finish(status.as_ref().is_ok_and(std::process::ExitStatus::success));
                let status = status.expect("run cooking browser E2E");
                assert!(status.success(), "cooking browser E2E failed: {status}");
                if let Some(installed_uis) = installed_reference_uis {
                    let audit_actor_id = user_id.as_uuid();
                    let audit_organization_id = organization_id.as_uuid();
                    let static_installation_id = installed_uis.static_ui.installation_id;
                    let static_generation_id = installed_uis.static_ui.generation_id;
                    let managed_installation_id = installed_uis.managed_ui.installation_id;
                    let managed_generation_id = installed_uis.managed_ui.generation_id;
                    assert_installed_ui_audit_success(
                        &pool,
                        audit_actor_id,
                        audit_organization_id,
                        static_installation_id,
                        static_generation_id,
                        UiRequestAuditSurface::HandoffIssue,
                        false,
                    )
                    .await;
                    assert_installed_ui_audit_success(
                        &pool,
                        audit_actor_id,
                        audit_organization_id,
                        static_installation_id,
                        static_generation_id,
                        UiRequestAuditSurface::HandoffExchange,
                        true,
                    )
                    .await;
                    assert_installed_ui_audit_success(
                        &pool,
                        audit_actor_id,
                        audit_organization_id,
                        static_installation_id,
                        static_generation_id,
                        UiRequestAuditSurface::Bootstrap,
                        true,
                    )
                    .await;
                    assert_installed_ui_audit_success(
                        &pool,
                        audit_actor_id,
                        audit_organization_id,
                        static_installation_id,
                        static_generation_id,
                        UiRequestAuditSurface::Static,
                        true,
                    )
                    .await;
                    assert_installed_ui_audit_success(
                        &pool,
                        audit_actor_id,
                        audit_organization_id,
                        managed_installation_id,
                        managed_generation_id,
                        UiRequestAuditSurface::HandoffIssue,
                        false,
                    )
                    .await;
                    assert_installed_ui_audit_success(
                        &pool,
                        audit_actor_id,
                        audit_organization_id,
                        managed_installation_id,
                        managed_generation_id,
                        UiRequestAuditSurface::HandoffExchange,
                        true,
                    )
                    .await;
                    assert_installed_ui_audit_success(
                        &pool,
                        audit_actor_id,
                        audit_organization_id,
                        managed_installation_id,
                        managed_generation_id,
                        UiRequestAuditSurface::Bootstrap,
                        true,
                    )
                    .await;
                    assert_installed_ui_audit_success(
                        &pool,
                        audit_actor_id,
                        audit_organization_id,
                        managed_installation_id,
                        managed_generation_id,
                        UiRequestAuditSurface::Managed,
                        true,
                    )
                    .await;
                    let api_request_ids = assert_installed_ui_audit_success(
                        &pool,
                        audit_actor_id,
                        audit_organization_id,
                        managed_installation_id,
                        managed_generation_id,
                        UiRequestAuditSurface::Api,
                        true,
                    )
                    .await;
                    assert_installed_ui_audit_success(
                        &pool,
                        audit_actor_id,
                        audit_organization_id,
                        managed_installation_id,
                        managed_generation_id,
                        UiRequestAuditSurface::Embed,
                        false,
                    )
                    .await;
                    let correlated_api_count: i64 = sqlx::query_scalar(
                        "SELECT count(*)
                           FROM gateway_invocations invocation
                           JOIN ui_request_audit_events audit
                             ON audit.request_id = invocation.request_id
                          WHERE audit.installation_id = $1
                            AND audit.generation_id = $2
                            AND audit.surface = $3
                            AND audit.request_id = ANY($4)",
                    )
                    .bind(managed_installation_id)
                    .bind(managed_generation_id)
                    .bind(UiRequestAuditSurface::Api.as_str())
                    .bind(&api_request_ids)
                    .fetch_one(&pool)
                    .await
                    .expect("read managed UI gateway invocation correlation");
                    assert!(
                        correlated_api_count > 0,
                        "managed API audit must correlate to a gateway invocation"
                    );
                    println!(
                        "REAL_UI_INSTALLATION_AUDIT=1 static=1 managed=1 api=1 embed=1 gateway_correlation=1"
                    );
                }
                let browser_gateway_id = installed_gateway.gateway_id;
                let active_revision_id: uuid::Uuid =
                    sqlx::query_scalar("SELECT active_revision_id FROM gateways WHERE id = $1")
                        .bind(browser_gateway_id)
                        .fetch_one(&pool)
                        .await
                        .expect("cooking browser active gateway revision");
                assert_ne!(
                    active_revision_id, installed_gateway.revision_id,
                    "browser must create a new gateway revision"
                );
                actual_grant_id = Some(
                    sqlx::query_scalar(
                        "SELECT binding_grant.id
                     FROM gateways gateway
                     JOIN gateway_revisions revision
                       ON revision.gateway_id = gateway.id
                      AND revision.id = gateway.active_revision_id
                     JOIN gateway_mailbox_bindings binding
                       ON binding.gateway_revision_id = revision.id
                      AND binding.mailbox_id = $1
                     JOIN gateway_mailbox_binding_grants binding_grant
                       ON binding_grant.binding_id = binding.id
                      AND binding_grant.status = 'active'
                     WHERE gateway.id = $2
                     ORDER BY binding_grant.granted_at DESC, binding_grant.id DESC
                         LIMIT 1",
                    )
                    .bind(instance.mailbox_id)
                    .bind(browser_gateway_id)
                    .fetch_one(&pool)
                    .await
                    .expect("cooking browser active binding grant"),
                );
            }
        }
        let mut actual_fixture = GatewayGoldenFixture {
            mailbox_id: MailboxId::from_uuid(instance.mailbox_id),
            grant_id: actual_grant_id.expect("cooking gateway mailbox grant after setup"),
        };
        // Run the adversarial release only after the optional browser phase:
        // the browser is allowed to install/configure the canonical release,
        // and must not accidentally make this authority probe a no-op.
        let adversarial_installed_gateway = cooking_builds::install_cooking_gateway(
            &cooking_context,
            adversarial_gateway_build.release_id,
            adversarial_gateway_build.repository_id,
        )
        .await
        .expect("install adversarial foreign-slot gateway release");
        let adversarial_configured = cooking_builds::configure_cooking_gateway(
            &cooking_context,
            adversarial_installed_gateway,
            cooking_builds::cooking_gateway_parameters(&cooking_inbound_placeholder, 1001, 1002),
            actual_brokered.import_id,
            actual_brokered.version_id,
            foreign_instance.mailbox_id,
        )
        .await
        .expect("configure adversarial foreign publication slot");
        let installed_release: uuid::Uuid =
            sqlx::query_scalar("SELECT release_id FROM gateway_revisions WHERE id = $1")
                .bind(adversarial_configured.revision_id)
                .fetch_one(&pool)
                .await
                .expect("adversarial revision release provenance");
        assert_eq!(installed_release, adversarial_gateway_build.release_id);
        let has_foreign_binding: bool = sqlx::query_scalar(
            "SELECT EXISTS (
                 SELECT 1 FROM gateway_mailbox_bindings binding
                 JOIN gateway_mailbox_binding_grants grant_row
                   ON grant_row.binding_id = binding.id
                 WHERE binding.gateway_revision_id = $1
                   AND binding.slot_key = 'cooking_requests'
                   AND binding.mailbox_id = $2
                   AND grant_row.status = 'active'
             )",
        )
        .bind(adversarial_configured.revision_id)
        .bind(foreign_instance.mailbox_id)
        .fetch_one(&pool)
        .await
        .expect("adversarial foreign mailbox binding");
        assert!(
            has_foreign_binding,
            "foreign mailbox positive control binding"
        );
        let adversarial_foreign_mailbox = foreign_instance.mailbox_id;
        let dispatcher = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("reserve cooking gateway dispatcher listener");
        let dispatcher_listen = dispatcher
            .local_addr()
            .expect("cooking gateway dispatcher listener address");
        drop(dispatcher);
        app_config.gateway_edge = Some(GatewayEdgeConfig {
            caddy_admin_url: env::var("HEPHAESTUS_CADDY_TEST_ADMIN_URL")
                .expect("joined Caddy admin URL"),
            caddy_configuration_template: caddy_configuration(
                &env::var("HEPHAESTUS_CADDY_TEST_ADMIN_URL").expect("joined Caddy admin URL"),
            ),
            caddy_server_name: String::from("shared"),
            dispatcher_listen,
            public_authority: String::from("gateway.golden.invalid"),
            ui_origin: None,
        });
        app_config.secret_broker_adapter = actual_brokered.upstream.adapter();
        // Finish the separate adversarial instance's immutable bindings and
        // rules before the daemon restart. The later ingress uses the
        // restarted daemon, but this setup remains tied to the published
        // release and final pre-restart revision.
        let canonical_revision_id: uuid::Uuid = sqlx::query_scalar(
            "SELECT active_revision_id
               FROM agent_instances
              WHERE id = $1",
        )
        .bind(actual_instance.instance)
        .fetch_one(&pool)
        .await
        .expect("canonical active instance revision");
        let (adversarial_instance, adversarial_rule_id) =
            cooking_adversarial_agent::prepare_adversarial_instance(
                &cooking_context,
                adversarial_agent_build.release_agent_id,
                blog_repository.repository_id,
                canonical_revision_id,
                "cooking-agent-adversarial",
                "cooking-agent-adversarial",
            )
            .await
            .expect("prepare adversarial cooking agent bindings and rules");
        cooking_builds::wait_for_cooking_build_quiescence(
            &pool,
            project.id,
            "golden-cooking-oci-materialization",
            cooking_wait_timeout,
        )
        .await;
        running
            .shutdown()
            .await
            .expect("build-proof daemon restart shutdown");
        let running = Box::pin(restart_application(app_config.clone())).await;
        let adversarial_url = format!(
            "{}/gateway/cooking/telegram",
            env::var("HEPHAESTUS_CADDY_TEST_PUBLIC_URL").expect("public Caddy URL")
        );
        let adversarial_response = reqwest::Client::new()
            .post(adversarial_url)
            .header("x-telegram-bot-api-secret-token", cooking::INBOUND_SENTINEL)
            .json(&serde_json::json!({
                "update_id": 39,
                "message": {"from": {"id": 1001}, "text": "pasta"}
            }))
            .send()
            .await
            .expect("adversarial foreign publication request");
        assert_eq!(
            adversarial_response.status(),
            reqwest::StatusCode::BAD_GATEWAY,
            "undeclared gateway publication slot is denied by the host"
        );
        adversarial_response
            .bytes()
            .await
            .expect("read adversarial denial response");
        let denied_publications: Vec<(String, Option<String>, String)> = sqlx::query_as(
            "SELECT publication.outcome, publication.denial_code, invocation.outcome
               FROM gateway_mailbox_publications publication
               JOIN gateway_invocations invocation ON invocation.id = publication.invocation_id
              WHERE publication.gateway_revision_id = $1
                AND publication.slot_key = $2
                AND publication.deduplication_key = 'telegram-update-39'",
        )
        .bind(adversarial_configured.revision_id)
        .bind(cooking_builds::FOREIGN_PUBLICATION_SLOT)
        .fetch_all(&pool)
        .await
        .expect("exact adversarial publication denial");
        assert_eq!(
            denied_publications,
            vec![(
                String::from("denied"),
                Some(String::from("authority_unavailable")),
                String::from("failed"),
            )],
            "the guest must reach publication and receive a durable authority denial"
        );
        let (foreign_events, foreign_deliveries, foreign_attempts, foreign_runs): (
            i64,
            i64,
            i64,
            i64,
        ) = sqlx::query_as(
            "SELECT
                (SELECT count(*) FROM mailbox_events WHERE mailbox_id = $1),
                (SELECT count(*) FROM mailbox_deliveries WHERE mailbox_id = $1),
                (SELECT count(*) FROM mailbox_delivery_attempts WHERE mailbox_id = $1),
                (SELECT count(*) FROM runs WHERE instance_id = (
                    SELECT instance_id FROM mailboxes WHERE id = $1
                ))",
        )
        .bind(adversarial_foreign_mailbox)
        .fetch_one(&pool)
        .await
        .expect("foreign mailbox denial effects");
        assert_eq!(
            (
                foreign_events,
                foreign_deliveries,
                foreign_attempts,
                foreign_runs
            ),
            (0, 0, 0, 0),
            "foreign mailbox must have no event, delivery, attempt, or run after denial"
        );
        let restored_context = cooking_builds::CookingBuildContext {
            pool: &pool,
            running: &running,
            root: &root,
            source_root: &source_root,
            project_id: project.id,
            repositories: &fixture_repository,
            identity: cooking_builds::CookingIdentity {
                actor: &identity,
                git_token: &token,
                rpc_token: &rpc_token,
            },
            timeout: cooking_wait_timeout,
        };
        let canonical_installed_gateway = cooking_builds::install_cooking_gateway(
            &restored_context,
            builds.gateway.release_id,
            builds.gateway.repository_id,
        )
        .await
        .expect("reinstall canonical gateway after adversarial probe");
        let restored = cooking_builds::configure_cooking_gateway(
            &restored_context,
            canonical_installed_gateway,
            cooking_builds::cooking_gateway_parameters(&cooking_inbound_placeholder, 1001, 1002),
            actual_brokered.import_id,
            actual_brokered.version_id,
            instance.mailbox_id,
        )
        .await
        .expect("restore canonical gateway after adversarial probe");
        actual_fixture.grant_id = restored.grant_id;
        let checkpoint = cooking::exercise_initial(&pool, &actual_fixture).await;
        cooking::wait_for_checkpoint_runs(&pool, &checkpoint, restored_context.timeout).await;
        let canonical_run_ids = checkpoint.run_ids();
        // Recipe 42 above is the canonical positive control. Capture its
        // durable adapter effects before routing one mailbox-local event to a
        // separate adversarial instance. Scope both measurements to these
        // settled run IDs so unrelated canonical activity cannot move the
        // comparison window.
        let (baseline_model_calls, baseline_relay_calls): (i64, i64) = sqlx::query_as(
            "SELECT
                 count(*) FILTER (WHERE audit.rule_id = $2),
                 count(*) FILTER (WHERE audit.rule_id = $3)
               FROM brokered_secret_audit_events AS audit
               JOIN runs AS run ON run.id = audit.run_id
              WHERE run.id = ANY($1)
                AND audit.event_kind = 'substitution_use'",
        )
        .bind(canonical_run_ids.to_vec())
        .bind(cooking::MODEL_RULE)
        .bind(cooking::RELAY_RULE)
        .fetch_one(&pool)
        .await
        .expect("canonical cooking adapter baseline");
        let (gateway_id, gateway_revision_id): (uuid::Uuid, uuid::Uuid) = sqlx::query_as(
            "SELECT gateway.id, gateway.active_revision_id
               FROM gateways AS gateway
               JOIN gateway_revisions AS revision
                 ON revision.gateway_id = gateway.id
                AND revision.id = gateway.active_revision_id
               JOIN gateway_mailbox_bindings AS binding
                 ON binding.gateway_revision_id = revision.id
               JOIN gateway_mailbox_binding_grants AS grant_row
                 ON grant_row.binding_id = binding.id
              WHERE grant_row.id = $1
                AND grant_row.status = 'active'",
        )
        .bind(actual_fixture.grant_id)
        .fetch_one(&pool)
        .await
        .expect("canonical active gateway identity");
        let adversarial_probe = cooking_adversarial_agent::exercise_adversarial_agent_probe(
            cooking_adversarial_agent::AdversarialAgentProbeInput {
                context: &restored_context,
                gateway: cooking_builds::InstalledCookingGateway {
                    gateway_id,
                    revision_id: gateway_revision_id,
                },
                adversarial_instance,
                canonical_run_ids,
                baseline_model_calls,
                baseline_relay_calls,
                adversarial_rule_id,
                inbound_import_id: actual_brokered.import_id,
                inbound_version_id: actual_brokered.version_id,
                inbound_placeholder: &cooking_inbound_placeholder,
                inbound_wire_credential: cooking::INBOUND_SENTINEL,
                public_url: &env::var("HEPHAESTUS_CADDY_TEST_PUBLIC_URL")
                    .expect("joined Caddy public URL"),
            },
        )
        .await
        .expect("adversarial cooking agent denial probe");
        assert!(!adversarial_probe.event_id.is_nil());
        assert!(!adversarial_probe.run_id.is_nil());
        assert_eq!(adversarial_probe.mismatched_rule_id, adversarial_rule_id);
        assert_eq!(adversarial_probe.deny_decisions, 1);
        assert_eq!(adversarial_probe.substitution_uses, 0);
        let restored_adversarial_gateway = cooking_builds::install_cooking_gateway(
            &restored_context,
            builds.gateway.release_id,
            builds.gateway.repository_id,
        )
        .await
        .expect("reinstall canonical gateway after agent probe");
        let restored_adversarial = cooking_builds::configure_cooking_gateway(
            &restored_context,
            restored_adversarial_gateway,
            cooking_builds::cooking_gateway_parameters(&cooking_inbound_placeholder, 1001, 1002),
            actual_brokered.import_id,
            actual_brokered.version_id,
            instance.mailbox_id,
        )
        .await
        .expect("restore canonical gateway after agent probe");
        actual_fixture.grant_id = restored_adversarial.grant_id;
        // Prepare the transformed guest and its two new rule identities before
        // the existing supervisor restart. This follows the initial and
        // adversarial controls, while canonical/crash adapters are joined by
        // immutable rule UUID and canonical counts remain unchanged.
        let crash_agent_build =
            cooking_builds::build_and_publish_guest_crash_agent(&restored_context, &builds.agent)
                .await
                .expect("publish deterministic guest crash agent");
        let crash_instance = cooking_adversarial_agent::prepare_brokered_instance_with_rule_ids(
            &restored_context,
            crash_agent_build.release_agent_id,
            blog_repository.repository_id,
            canonical_revision_id,
            "cooking-agent-guest-crash",
            "cooking-agent-guest-crash",
            cooking_adversarial_agent::BrokeredRuleIds {
                model: cooking::CRASH_MODEL_RULE,
                relay: cooking::CRASH_RELAY_RULE,
            },
        )
        .await
        .expect("prepare guest crash instance and rule specs");
        let crash_upstream = cooking::cooking_crash_upstreams(vec![
            cooking::brokered_rule_for_spec(&crash_instance.model),
            cooking::brokered_rule_for_spec(&crash_instance.relay),
        ])
        .await;
        let crash_gateway_id: uuid::Uuid =
            sqlx::query_scalar("SELECT gateway_id FROM gateway_revisions WHERE id = $1")
                .bind(restored_adversarial.revision_id)
                .fetch_one(&pool)
                .await
                .expect("guest crash gateway identity");
        let crash_gateway = cooking_builds::configure_cooking_gateway(
            &restored_context,
            cooking_builds::InstalledCookingGateway {
                gateway_id: crash_gateway_id,
                revision_id: restored_adversarial.revision_id,
            },
            cooking_builds::cooking_gateway_parameters(&cooking_inbound_placeholder, 1001, 1002),
            actual_brokered.import_id,
            actual_brokered.version_id,
            crash_instance.instance.mailbox_id,
        )
        .await
        .expect("route cooking gateway to guest crash mailbox");
        app_config.secret_broker_adapter =
            actual_brokered.upstream.combined_adapter(&crash_upstream);
        cooking_builds::wait_for_cooking_build_quiescence(
            &pool,
            project.id,
            "golden-cooking-oci-materialization",
            cooking_wait_timeout,
        )
        .await;
        running
            .shutdown()
            .await
            .expect("cooking daemon graceful restart shutdown");
        let restarted = Box::pin(restart_application(app_config)).await;
        let crash_fixture = GatewayGoldenFixture {
            mailbox_id: mailbox_domain::MailboxId::from_uuid(crash_instance.instance.mailbox_id),
            grant_id: crash_gateway.grant_id,
        };
        cooking_guest_crash::exercise(&pool, &crash_fixture, &crash_instance.instance).await;
        cooking_guest_crash::wait_for_upstream(crash_upstream).await;
        let crash_disk: PathBuf = PathBuf::from(
            sqlx::query_scalar::<_, String>(
                "SELECT host_path FROM agent_instance_state_volumes
              WHERE instance_id = $1",
            )
            .bind(crash_instance.instance.instance_id)
            .fetch_one(&pool)
            .await
            .expect("load guest crash state volume disk path"),
        );
        let restarted_context = cooking_builds::CookingBuildContext {
            pool: &pool,
            running: &restarted,
            root: &root,
            source_root: &source_root,
            project_id: project.id,
            repositories: &fixture_repository,
            identity: cooking_builds::CookingIdentity {
                actor: &identity,
                git_token: &token,
                rpc_token: &rpc_token,
            },
            timeout: cooking_wait_timeout,
        };
        let restored_after_crash = cooking_builds::configure_cooking_gateway(
            &restarted_context,
            cooking_builds::InstalledCookingGateway {
                gateway_id: crash_gateway_id,
                revision_id: crash_gateway.revision_id,
            },
            cooking_builds::cooking_gateway_parameters(&cooking_inbound_placeholder, 1001, 1002),
            actual_brokered.import_id,
            actual_brokered.version_id,
            instance.mailbox_id,
        )
        .await
        .expect("restore canonical gateway after guest crash branch");
        actual_fixture.grant_id = restored_after_crash.grant_id;
        let resolved_head = cooking::exercise_follow_up(
            &pool,
            &restarted,
            &actual_instance,
            &actual_fixture,
            &root,
            blog_repository.repository_id.as_uuid(),
            &blog_repository.source_commit,
            checkpoint,
            &actual_brokered.upstream,
        )
        .await;
        let blog_artifact = cooking_blog_artifact::build_publish_and_verify(
            &cooking_builds::CookingBuildContext {
                pool: &pool,
                running: &restarted,
                root: &root,
                source_root: &source_root,
                project_id: project.id,
                repositories: &fixture_repository,
                identity: cooking_builds::CookingIdentity {
                    actor: &identity,
                    git_token: &token,
                    rpc_token: &rpc_token,
                },
                timeout: cooking_wait_timeout,
            },
            &blog_repository,
            &resolved_head,
            "Family pasta",
            outsider_id,
        )
        .await
        .expect("build and retrieve published cooking blog artifact");
        assert_eq!(blog_artifact.source_commit, resolved_head);
        assert!(!blog_artifact.build_id.is_nil());
        assert!(!blog_artifact.release_id.is_nil());
        assert!(!blog_artifact.artifact_id.is_nil());
        assert_eq!(blog_artifact.sha256.len(), 64);
        eprintln!(
            "cooking blog artifact: source_commit={}, build={}, release={}, artifact={}, sha256={}",
            blog_artifact.source_commit,
            blog_artifact.build_id,
            blog_artifact.release_id,
            blog_artifact.artifact_id,
            blog_artifact.sha256
        );
        let mut update_state_volume_disk: Option<PathBuf> = None;
        let browser_abnormal_release_agent_id = update_builds
            .as_ref()
            .map(|builds| builds.abnormal.release_agent_id);
        let update_sequence = if let Some(update_builds) = update_builds {
            let current_revision_id: uuid::Uuid =
                sqlx::query_scalar("SELECT active_revision_id FROM agent_instances WHERE id = $1")
                    .bind(actual_instance.instance)
                    .fetch_one(&pool)
                    .await
                    .expect("load configured cooking revision");
            let sequence = cooking_updates::exercise_barrier_update_sequence(
                &cooking_updates::CookingUpdateContext {
                    pool: &pool,
                    running: &restarted,
                    instance_id: actual_instance.instance,
                    gateway: &actual_fixture,
                    current_revision_id,
                    owner: user_id.as_uuid(),
                    rpc_token: &rpc_token,
                    brokered_rule_ids: cooking_updates::BrokeredRuleIds {
                        model: cooking::MODEL_RULE,
                        relay: cooking::RELAY_RULE,
                    },
                    timeout: cooking_wait_timeout,
                },
                &actual_brokered.upstream,
                cooking_updates::CookingUpdateCandidates {
                    migrate_release_agent_id: update_builds.migrate.release_agent_id,
                    rollback_release_agent_id: update_builds.rollback.release_agent_id,
                    abnormal_release_agent_id: update_builds.abnormal.release_agent_id,
                    migrate_rule_ids: cooking_update_rule_ids
                        .expect("update rule IDs allocated with update builds")
                        .0,
                    rollback_rule_ids: cooking_update_rule_ids
                        .expect("update rule IDs allocated with update builds")
                        .1,
                    abnormal_rule_ids: cooking_update_rule_ids
                        .expect("update rule IDs allocated with update builds")
                        .2,
                },
            )
            .await;
            let disk: String = sqlx::query_scalar(
                "SELECT host_path FROM agent_instance_state_volumes
                 WHERE instance_id = $1",
            )
            .bind(actual_instance.instance)
            .fetch_one(&pool)
            .await
            .expect("load cooking state volume disk path");
            update_state_volume_disk = Some(PathBuf::from(disk));
            Some(sequence)
        } else {
            None
        };
        if update_sequence.is_some() {
            // The update helper rotates relay while event 47 still holds the
            // v1 model lease, before CreateUpdate clones candidate rules.
            // Event 46 remains the completed old-version relay proof.
            let sequence = update_sequence.as_ref().expect("cooking update sequence");
            let relay_run_id = sequence.relay_run_id;
            let relay_rotation = sequence.relay_rotation;
            let inbound_rotation = cooking::rotate_inbound_credential(
                &pool,
                user_id,
                actual_brokered.import_id,
                actual_brokered.version_id,
            )
            .await;
            actual_fixture = cooking_authority::configure_rotated_inbound_gateway(
                &pool,
                &restarted,
                &actual_fixture,
                &rpc_token,
                actual_brokered.import_id,
                inbound_rotation.rotated_version_id,
                instance.mailbox_id,
            )
            .await
            .expect("configure rotated cooking inbound credential");
            let public =
                env::var("HEPHAESTUS_CADDY_TEST_PUBLIC_URL").expect("joined Caddy public URL");
            let url = format!("{public}/gateway/cooking/telegram");
            let client = reqwest::Client::new();
            let old = cooking::send_update_with_credential(
                &client,
                &url,
                49,
                1001,
                "rotation",
                cooking::INBOUND_SENTINEL,
            )
            .await;
            assert_eq!(old.status(), reqwest::StatusCode::UNAUTHORIZED);
            old.bytes().await.expect("old inbound rotation denial");
            let accepted = cooking::send_update_with_credential(
                &client,
                &url,
                49,
                1001,
                "rotation",
                cooking::INBOUND_ROTATED_SENTINEL,
            )
            .await;
            assert_eq!(accepted.status(), reqwest::StatusCode::OK);
            accepted
                .bytes()
                .await
                .expect("rotated inbound acknowledgement");
            let rotated_run = cooking::wait_for_event_run(&pool, &actual_fixture, 49).await;
            cooking::assert_rotated_brokered_lease(
                &pool,
                relay_run_id,
                rotated_run.event_id,
                cooking::RELAY_RULE,
                sequence.migration.relay_rule_id,
                relay_rotation,
            )
            .await;
            cooking_authority::assert_inbound_lease_history(
                &pool,
                &actual_fixture,
                inbound_rotation.pinned_version_id,
                inbound_rotation.rotated_version_id,
            )
            .await
            .expect("inbound lease rotation history");
        }
        if browser_e2e {
            let (completed_run_id, result_commit, result_ref, target_ref): (
                uuid::Uuid,
                String,
                String,
                String,
            ) = sqlx::query_as(
                "SELECT run.id, result.result_commit, result.result_ref,
                        proposal.target_ref
                   FROM runs run
                   JOIN run_results result ON result.run_id = run.id
                   JOIN review_proposals proposal ON proposal.run_id = run.id
                  WHERE run.instance_id = $1 AND result.state = 'completed'
                    AND proposal.state = 'approved'
                    AND result.result_commit IS NOT NULL
                  ORDER BY result.completed_at DESC, result.id DESC
                  LIMIT 1",
            )
            .bind(actual_instance.instance)
            .fetch_one(&pool)
            .await
            .expect("completed cooking result for browser inspection");
            // The browser must exercise the approval command against a real
            // completed proposal.  The fault-recovery requests in the joined
            // scenario intentionally remain open after their provenance
            // inspection, so select the newest such proposal instead of
            // fabricating a pending row for the UI fixture.
            let (pending_run_id, pending_proposal_id): (uuid::Uuid, uuid::Uuid) = sqlx::query_as(
                "SELECT run.id, proposal.id
                       FROM runs run
                       JOIN review_proposals proposal ON proposal.run_id = run.id
                      WHERE run.instance_id = $1
                        AND proposal.state IN ('open', 'approval_requested')
                        AND EXISTS (
                            SELECT 1 FROM run_results result
                             WHERE result.run_id = run.id
                               AND result.state = 'completed'
                               AND result.result_commit IS NOT NULL
                        )
                      ORDER BY proposal.created_at DESC, proposal.id DESC
                      LIMIT 1",
            )
            .bind(actual_instance.instance)
            .fetch_one(&pool)
            .await
            .expect("open cooking result proposal for browser approval");
            let proposal_id: uuid::Uuid = sqlx::query_scalar(
                "SELECT id FROM review_proposals WHERE run_id = $1 ORDER BY id LIMIT 1",
            )
            .bind(completed_run_id)
            .fetch_one(&pool)
            .await
            .expect("completed cooking review proposal");
            let retry_run_ids_before: Vec<uuid::Uuid> = sqlx::query_scalar(
                "SELECT run_id FROM run_requests WHERE retry_of_run_id = $1 ORDER BY run_id",
            )
            .bind(
                retry_fixture
                    .as_ref()
                    .expect("browser retry source run fixture")
                    .source_run_id,
            )
            .fetch_all(&pool)
            .await
            .expect("browser retry ancestry before browser actions");
            let post_path = format!(
                "{}.post.json",
                env::var("HEPHAESTUS_COOKING_BROWSER_FIXTURE_OUTPUT")
                    .expect("cooking browser fixture output path")
            );
            let post_fixture = serde_json::json!({
                "organization_id": organization_id,
                "project_id": project.id,
                "repository_id": builds.gateway.repository_id,
                "release_id": builds.gateway.release_id,
                "release_agent_id": builds.agent.release_agent_id,
                "instance_id": instance.instance_id,
                "mailbox_id": instance.mailbox_id,
                "gateway_id": sqlx::query_scalar::<_, uuid::Uuid>(
                    "SELECT binding.gateway_id
                       FROM gateway_mailbox_bindings binding
                       JOIN gateway_mailbox_binding_grants grant_row
                         ON grant_row.binding_id = binding.id
                      WHERE grant_row.id = $1",
                )
                .bind(actual_fixture.grant_id)
                .fetch_one(&pool)
                .await
                .expect("post-operation cooking gateway id"),
                "completed_run_id": completed_run_id,
                "result_commit": result_commit,
                "result_ref": result_ref,
                "target_ref": target_ref,
                "proposal_id": proposal_id,
                "pending_run_id": pending_run_id,
                "pending_proposal_id": pending_proposal_id,
                "retry_source_run_id": retry_fixture
                    .as_ref()
                    .expect("browser retry source run fixture")
                    .source_run_id,
                "migration_update_id": update_sequence.as_ref().map(|s| s.migration.update_id),
                "abnormal_update_id": update_sequence.as_ref().map(|s| s.abnormal.update_id),
                "abnormal_candidate_revision_id": update_sequence
                    .as_ref()
                    .map(|s| s.abnormal.candidate_revision_id),
                "abnormal_release_agent_id": browser_abnormal_release_agent_id,
                "browser_model_rule_id": cooking_update_rule_ids
                    .map(|(_, _, _, browser)| browser.model),
                "browser_relay_rule_id": cooking_update_rule_ids
                    .map(|(_, _, _, browser)| browser.relay),
                "browser_rule_copies": cooking_update_rule_ids.map(|(migration, _, _, browser)| {
                    vec![
                        serde_json::json!({
                            "source_rule_id": migration.model,
                            "candidate_rule_id": browser.model,
                        }),
                        serde_json::json!({
                            "source_rule_id": migration.relay,
                            "candidate_rule_id": browser.relay,
                        }),
                    ]
                }),
            });
            tokio::fs::write(
                &post_path,
                serde_json::to_vec_pretty(&post_fixture)
                    .expect("post-operation browser fixture JSON"),
            )
            .await
            .expect("write post-operation browser fixture JSON");
            let issuer = env::var("HEPHAESTUS_COOKING_BROWSER_OIDC_ISSUER")
                .expect("cooking browser OIDC issuer");
            let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../scripts/run-ui-e2e-external.sh");
            let browser_timer =
                WorkloadPhaseTimer::start("browser-post-operation", workload_phase_timing);
            let status = tokio::process::Command::new(script)
                .env("HEPHAESTUS_E2E_COOKING_FIXTURE", &post_path)
                .env("HEPHAESTUS_E2E_EXTERNAL_DATABASE_URL", &database_url)
                .env(
                    "HEPHAESTUS_E2E_EXTERNAL_RPC_ENDPOINT",
                    restarted.http_addr().to_string(),
                )
                .env(
                    "HEPHAESTUS_E2E_EXTERNAL_RPC_SECRET",
                    "golden-internal-command-token-with-sufficient-entropy",
                )
                .env("HEPHAESTUS_E2E_EXTERNAL_OIDC_ISSUER", issuer)
                .env("HEPHAESTUS_E2E_COOKING_PHASE", "post-operation")
                .env("HEPHAESTUS_E2E_BROWSER_RUNNER", "legacy")
                .status()
                .await;
            browser_timer.finish(status.as_ref().is_ok_and(std::process::ExitStatus::success));
            let status = status.expect("run cooking post-operation browser E2E");
            assert!(
                status.success(),
                "cooking post-operation browser E2E failed: {status}"
            );
            // Retry is a separate run command. Do not tear down the daemon
            // while its guest is still cleaning up; otherwise the subsequent
            // NATS and state-volume scans could miss a real active resource.
            let retry_run_ids_after: Vec<uuid::Uuid> = sqlx::query_scalar(
                "SELECT run_id FROM run_requests WHERE retry_of_run_id = $1 ORDER BY run_id",
            )
            .bind(
                retry_fixture
                    .as_ref()
                    .expect("browser retry source run fixture")
                    .source_run_id,
            )
            .fetch_all(&pool)
            .await
            .expect("browser retry ancestry after browser actions");
            assert_eq!(
                retry_run_ids_after.len(),
                retry_run_ids_before.len() + 1,
                "browser retry creates exactly one request for the selected source run"
            );
            assert!(
                retry_run_ids_before
                    .iter()
                    .all(|id| retry_run_ids_after.contains(id))
            );
            let new_retry_ids: Vec<_> = retry_run_ids_after
                .iter()
                .filter(|id| !retry_run_ids_before.contains(id))
                .copied()
                .collect();
            assert_eq!(new_retry_ids.len(), 1);
            let retry_run_id = new_retry_ids[0];
            let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
            loop {
                let (state, outcome): (String, Option<String>) =
                    sqlx::query_as("SELECT state, outcome FROM runs WHERE id = $1")
                        .bind(retry_run_id)
                        .fetch_one(&pool)
                        .await
                        .expect("poll exact browser retry run");
                if state == "cleaned_up" {
                    assert_eq!(
                        outcome.as_deref(),
                        Some("succeeded"),
                        "browser retry succeeds for the dedicated forge source"
                    );
                    break;
                }
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "browser retry run did not reach terminal cleanup"
                );
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            if let Some(sequence) = update_sequence {
                let abnormal_release_agent_id =
                    browser_abnormal_release_agent_id.expect("browser abnormal release agent");
                let state: (String, String, bool) = sqlx::query_as(
                    "SELECT update_record.state, instance.state, instance.run_gate_open
                       FROM agent_updates update_record
                       JOIN agent_instances instance ON instance.id = update_record.instance_id
                      WHERE update_record.id = $1",
                )
                .bind(sequence.abnormal.update_id)
                .fetch_one(&pool)
                .await
                .expect("browser recovery update state");
                assert_eq!(
                    state,
                    (
                        String::from("rejected"),
                        String::from("update_rejected"),
                        true
                    )
                );
                let browser_updates: i64 = sqlx::query_scalar(
                    "SELECT count(*)
                       FROM agent_updates update_record
                       JOIN agent_instance_revisions candidate
                         ON candidate.id = update_record.candidate_revision_id
                      WHERE update_record.instance_id = $1
                        AND candidate.release_agent_id = $2
                        AND update_record.state = 'rejected'",
                )
                .bind(actual_instance.instance)
                .bind(abnormal_release_agent_id)
                .fetch_one(&pool)
                .await
                .expect("browser-created abnormal update state");
                assert_eq!(
                    browser_updates, 2,
                    "both abnormal updates must be recovered"
                );
            }
        }
        if update_sequence.is_some() {
            cooking::exercise_active_relay_revocation(
                &pool,
                &actual_fixture,
                user_id,
                &actual_brokered.upstream,
                update_sequence
                    .as_ref()
                    .expect("cooking update sequence")
                    .migration
                    .relay_rule_id,
            )
            .await;
        }
        actual_brokered.upstream.assert_substituted_request().await;
        let inbound_credential = if update_sequence.is_some() {
            cooking::INBOUND_ROTATED_SENTINEL
        } else {
            cooking::INBOUND_SENTINEL
        };
        cooking_authority::retire_cooking_gateway_grant(
            &pool,
            &restarted,
            &actual_fixture,
            &rpc_token,
            inbound_credential,
            outsider_id,
        )
        .await
        .expect("retire cooking gateway mailbox grant through RPC");
        if update_sequence.is_some() {
            let retained_run_id: uuid::Uuid = sqlx::query_scalar(
                "SELECT id
                   FROM runs
                  WHERE instance_id = $1
                    AND run_kind = 'normal'
                    AND state = 'cleaned_up'
                    AND outcome = 'succeeded'
                  ORDER BY updated_at DESC, id DESC
                  LIMIT 1",
            )
            .bind(actual_instance.instance)
            .fetch_one(&pool)
            .await
            .expect("completed cooking run retained for retirement proof");
            let gateway_id: uuid::Uuid = sqlx::query_scalar(
                "SELECT gateway_id
                   FROM gateway_mailbox_bindings
                  WHERE id = (
                      SELECT binding_id
                        FROM gateway_mailbox_binding_grants
                       WHERE id = $1
                  )",
            )
            .bind(actual_fixture.grant_id)
            .fetch_one(&pool)
            .await
            .expect("cooking gateway identity for retirement proof");
            let outsider_identity = AuthenticatedIdentity::new(
                outsider_id,
                browser_oidc_issuer.clone(),
                "cooking-outsider",
                serde_json::json!({}),
                RequestId::new(),
            );
            let retirement_public = env::var("HEPHAESTUS_CADDY_TEST_PUBLIC_URL")
                .expect("joined Caddy public URL for retirement proof");
            let retry_fixture = retry_fixture
                .as_ref()
                .expect("retirement retry forge fixture");
            cooking_retirement::exercise(&cooking_retirement::RetirementContext {
                pool: &pool,
                running: &restarted,
                token_factory: &rpc_token,
                owner: &identity,
                outsider: &outsider_identity,
                instance: &actual_instance,
                retained_run_id,
                retry_instance: &retry_fixture.instance,
                retry_source_run_id: retry_fixture.source_run_id,
                retry_repository_id: retry_fixture.repository_id,
                gateway_id,
                project_id: project.id.as_uuid(),
                mailbox_id: instance.mailbox_id,
                public_url: &retirement_public,
                valid_inbound_credential: inbound_credential,
                import_parameters: cooking_builds::cooking_agent_parameters(),
            })
            .await
            .expect("retire cooking attachment, gateway, and release");
            // Source revocation follows grant retirement so the valid rotated
            // inbound credential proves the grant fence itself.
            cooking::revoke_imported_credential(&pool, user_id, actual_brokered.import_id).await;
        }
        cooking_confinement::assert_database_has_no_credentials(&pool).await;
        restarted.shutdown().await.expect("cooking daemon shutdown");
        let observer = observer.expect("cooking build proof VM observer");
        support::vm_observer_assertions::assert_cooking_vm_contracts(
            &pool,
            &observer,
            support::vm_observer_assertions::CookingVmContractIds {
                canonical_mailbox_id: instance.mailbox_id,
                crash_mailbox_id: crash_instance.instance.mailbox_id,
                adversarial_run_id: adversarial_probe.run_id,
                gateway_build_request_id: builds.gateway.build_request_id,
                agent_build_request_id: builds.agent.build_request_id,
                crash_agent_build_request_id: crash_agent_build.build_request_id,
            },
        )
        .await;
        cooking_guest_crash::assert_sqlite_disk(&crash_disk);
        cooking_confinement::assert_nats_has_no_credentials(&nats_url).await;
        if let Some(disk) = update_state_volume_disk {
            cooking_updates::assert_migrated_sqlite_disk(&disk, 9);
        }
        cleanup_streams(&nats_url).await;
        return;
    }
    let forge_source = create_forge_source_run(
        &pool,
        &running,
        &root,
        repository.id.as_uuid(),
        seeded_instance.instance,
        &token,
        libkrun_e2e,
        false,
        if cooking::enabled() {
            ForgeSourceContent::CookingBlog
        } else {
            ForgeSourceContent::GoldenAgent
        },
    )
    .await;
    let input_commit = forge_source.input_commit;
    let run_id = runtime_types::RunId::from_uuid(forge_source.run_id);

    if env::var("HEPHAESTUS_APP_UPDATE_ADMISSION_RACE_E2E").as_deref() == Ok("1") {
        #[cfg(feature = "test-fixtures")]
        {
            let admission_instance = support::rpc::update_admission::UpdateAdmissionInstance {
                instance_id: seeded_instance.instance,
                revision_id: seeded_instance.revision,
                release_id: seeded_instance.release,
                release_agent_id: seeded_instance.release_agent,
                attachment_id: seeded_instance.attachment,
            };
            let race = support::update_admission::exercise_reconciler_wins_race(
                &pool,
                &running,
                &admission_instance,
                user_id.as_uuid(),
                owner_browser_session,
            )
            .await;
            assert_eq!(race.initial_hook_run_id, race.retried_hook_run_id);
            assert_eq!(race.retried_hook_run_id, race.owner_recovery_hook_run_id);
            running
                .shutdown()
                .await
                .expect("app update race regression shutdown");
            cleanup_streams(&nats_url).await;
            return;
        }
        #[cfg(not(feature = "test-fixtures"))]
        assert_ne!(
            env::var("HEPHAESTUS_APP_UPDATE_ADMISSION_RACE_E2E").as_deref(),
            Ok("1"),
            "HEPHAESTUS_APP_UPDATE_ADMISSION_RACE_E2E requires --features hephaestus-app/test-fixtures"
        );
    }

    if env::var("HEPHAESTUS_APP_UPDATE_ADMISSION_E2E").as_deref() == Ok("1") {
        let admission_instance = support::rpc::update_admission::UpdateAdmissionInstance {
            instance_id: seeded_instance.instance,
            revision_id: seeded_instance.revision,
            release_id: seeded_instance.release,
            release_agent_id: seeded_instance.release_agent,
            attachment_id: seeded_instance.attachment,
        };
        let admission = support::update_admission::exercise(
            &pool,
            &running,
            &admission_instance,
            user_id.as_uuid(),
            owner_browser_session,
        )
        .await;
        assert_ne!(admission.update_id, uuid::Uuid::nil());
        assert_ne!(admission.initial_hook_run_id, admission.retried_hook_run_id);
        assert_ne!(
            admission.retried_hook_run_id,
            admission.owner_recovery_hook_run_id
        );
        running
            .shutdown()
            .await
            .expect("app update regression shutdown");
        cleanup_streams(&nats_url).await;
        return;
    }

    if cooking::enabled() {
        running
            .shutdown()
            .await
            .expect("cooking daemon startup recovery shutdown");
        let running = Box::pin(restart_application(app_config.clone())).await;
        let checkpoint =
            cooking::exercise_initial(&pool, &gateway_edge.as_ref().expect("cooking gateway").1)
                .await;
        running
            .shutdown()
            .await
            .expect("cooking daemon graceful restart shutdown");
        let restarted = Box::pin(restart_application(app_config)).await;
        let _ = cooking::exercise_follow_up(
            &pool,
            &restarted,
            &seeded_instance,
            &gateway_edge.as_ref().expect("cooking gateway").1,
            &root,
            repository.id.as_uuid(),
            &input_commit,
            checkpoint,
            &brokered_fixture.as_ref().expect("cooking broker").upstream,
        )
        .await;
        brokered_fixture
            .take()
            .expect("cooking broker")
            .upstream
            .assert_substituted_request()
            .await;
        restarted.shutdown().await.expect("cooking daemon shutdown");
        cleanup_streams(&nats_url).await;
        return;
    }

    if libkrun_e2e {
        let mut running = running;
        let (mut service_instance_id, mut service_resource_paths) = if gateway_caddy_e2e {
            let service_instance_id =
                if let Some(service_fixture) = gateway_service_fixture.as_ref() {
                    Some(wait_for_gateway_service_ready(&pool, service_fixture).await)
                } else {
                    None
                };
            let service_resource_paths = service_instance_id.map(|instance_id| {
                let vm_id = format!("gateway-service-{instance_id}");
                let provider_runtime_root = PathBuf::from(
                    env::var("HEPHAESTUS_LIBKRUN_RUNTIME_ROOT")
                        .expect("libkrun runtime root for cleanup assertion"),
                );
                let cgroup_root = PathBuf::from(
                    env::var("HEPHAESTUS_LIBKRUN_CGROUP_ROOT")
                        .expect("libkrun cgroup root for cleanup assertion"),
                );
                (
                    provider_runtime_root.join(&vm_id),
                    cgroup_root.join(&vm_id),
                    root.join("run-runtime")
                        .join("gateway-services")
                        .join(instance_id.to_string()),
                )
            });
            if let Some((provider_runtime, cgroup, materializer)) = &service_resource_paths {
                assert!(
                    provider_runtime.is_dir(),
                    "service VM runtime exists before shutdown"
                );
                assert!(cgroup.is_dir(), "service VM cgroup exists before shutdown");
                assert!(
                    materializer.is_dir(),
                    "service materializer tree exists before shutdown"
                );
            }
            (service_instance_id, service_resource_paths)
        } else {
            (None, None)
        };
        if gateway_caddy_e2e {
            let mut service_startup_id_before_crash = None;
            let admin_url =
                env::var("HEPHAESTUS_CADDY_TEST_ADMIN_URL").expect("joined Caddy admin URL");
            let applied_config =
                wait_for_caddy_configuration(&admin_url, "/gateway/brokered").await;
            assert!(
                applied_config.contains("/gateway/brokered"),
                "Caddy must contain the authoritative gateway route before the public proof: {applied_config}"
            );
            if gateway_service_fixture.is_some() {
                let applied_config =
                    wait_for_caddy_configuration(&admin_url, "/gateway/service").await;
                assert!(
                    applied_config.contains("/gateway/service"),
                    "Caddy must contain the persistent service route before the public proof: {applied_config}"
                );
                let first_service_proof = exercise_gateway_service_requests(
                    &env::var("HEPHAESTUS_CADDY_TEST_PUBLIC_URL").expect("joined Caddy public URL"),
                )
                .await;
                #[cfg(feature = "test-fixtures")]
                if gateway_service_log_guest_e2e {
                    let service_fixture = gateway_service_fixture
                        .as_ref()
                        .expect("persistent service fixture for guest log proof");
                    let first_instance_id = service_instance_id
                        .expect("persistent service instance for guest log proof");
                    let first_resource_paths = service_resource_paths
                        .as_ref()
                        .expect("persistent service paths for guest log proof");
                    let public_url = env::var("HEPHAESTUS_CADDY_TEST_PUBLIC_URL")
                        .expect("joined Caddy public URL for guest log proof");
                    let fencing_token: i64 = sqlx::query_scalar(
                        "SELECT fencing_token FROM gateway_service_instances
                          WHERE id = $1 AND gateway_id = $2 AND revision_id = $3",
                    )
                    .bind(first_instance_id)
                    .bind(service_fixture.gateway_id)
                    .bind(service_fixture.revision_id)
                    .fetch_one(&pool)
                    .await
                    .expect("read guest service-log fencing token");
                    let guest_log_proof = gateway_service_log_rpc::exercise_gateway_service_guest_log(
                        running.http_addr(),
                        service_fixture.gateway_id,
                        service_fixture.revision_id,
                        first_instance_id,
                        project.id.as_uuid(),
                        fencing_token,
                        user_id.as_uuid(),
                        owner_browser_session,
                        &public_url,
                    )
                    .await;
                    running
                        .shutdown()
                        .await
                        .expect("guest service-log daemon shutdown");
                    let first_state: Option<String> = sqlx::query_scalar(
                        "SELECT state FROM gateway_service_instances
                          WHERE id = $1 AND gateway_id = $2 AND revision_id = $3",
                    )
                    .bind(first_instance_id)
                    .bind(service_fixture.gateway_id)
                    .bind(service_fixture.revision_id)
                    .fetch_optional(&pool)
                    .await
                    .expect("read cleaned guest service-log instance");
                    assert_eq!(
                        first_state.as_deref(),
                        Some("cleaned"),
                        "guest service-log daemon shutdown must clean the service instance"
                    );
                    let (provider_runtime, cgroup, materializer) = first_resource_paths;
                    assert!(!provider_runtime.exists());
                    assert!(!cgroup.exists());
                    assert!(!materializer.exists());
                    assert_guest_gateway_service_log_retained(&pool, &guest_log_proof).await;
                    cleanup_streams(&nats_url).await;
                    println!(
                        "REAL_GATEWAY_SERVICE_LOG_GUEST_E2E=1 instance={first_instance_id} fence={} retained_after_shutdown=true",
                        guest_log_proof.fencing_token
                    );
                    return;
                }
                #[cfg(feature = "test-fixtures")]
                let app_pool_probe = if gateway_service_log_rpc_e2e {
                    let service_fixture = gateway_service_fixture
                        .as_ref()
                        .expect("persistent service fixture for log RPC");
                    let (app_pool, rpc_fixture) = prepare_gateway_service_log_rpc(
                        &pool,
                        &running,
                        service_fixture.gateway_id,
                        service_fixture.revision_id,
                        service_instance_id.expect("persistent service instance for log RPC"),
                        project.id.as_uuid(),
                    )
                    .await;
                    let member_id = rpc_fixture.member_id;
                    gateway_service_log_rpc::exercise_gateway_service_log_rpc(
                        &running,
                        rpc_fixture,
                        user_id.as_uuid(),
                        owner_browser_session,
                        outsider_id.as_uuid(),
                        outsider_browser_session,
                        || async {
                            sqlx::query(
                                "DELETE FROM project_maintainers WHERE project_id = $1 AND user_id = $2",
                            )
                            .bind(project.id.as_uuid())
                            .bind(member_id)
                            .execute(&pool)
                            .await
                            .expect("revoke service log RPC member");
                        },
                    )
                    .await;
                    Some(app_pool)
                } else {
                    None
                };
                service_startup_id_before_crash = Some(first_service_proof.startup_id.clone());
                if gateway_service_e2e {
                    let service_fixture = gateway_service_fixture
                        .as_ref()
                        .expect("persistent service fixture after first proof");
                    let first_instance_id =
                        service_instance_id.expect("persistent service instance after first proof");
                    let first_resource_paths = service_resource_paths
                        .as_ref()
                        .expect("persistent service paths after first proof");
                    running
                        .shutdown()
                        .await
                        .expect("first persistent service daemon shutdown");
                    #[cfg(feature = "test-fixtures")]
                    if let Some(app_pool_probe) = app_pool_probe {
                        assert!(
                            app_pool_probe.is_closed(),
                            "running daemon shutdown closes its application-role pool"
                        );
                    }
                    let (provider_runtime, cgroup, materializer) = first_resource_paths;
                    let first_state: Option<String> = sqlx::query_scalar(
                        "SELECT state FROM gateway_service_instances
                          WHERE id = $1 AND gateway_id = $2 AND revision_id = $3",
                    )
                    .bind(first_instance_id)
                    .bind(service_fixture.gateway_id)
                    .bind(service_fixture.revision_id)
                    .fetch_optional(&pool)
                    .await
                    .expect("read first cleaned persistent-service instance");
                    assert_eq!(
                        first_state.as_deref(),
                        Some("cleaned"),
                        "first daemon shutdown must clean the restored service instance"
                    );
                    assert!(!provider_runtime.exists());
                    assert!(!cgroup.exists());
                    assert!(!materializer.exists());

                    running = Box::pin(restart_application(app_config.clone())).await;
                    let second_instance_id =
                        wait_for_gateway_service_ready(&pool, service_fixture).await;
                    assert_ne!(
                        first_instance_id, second_instance_id,
                        "graceful daemon restart must create a new service instance"
                    );
                    let second_revision_id: uuid::Uuid = sqlx::query_scalar(
                        "SELECT revision_id FROM gateway_service_instances WHERE id = $1",
                    )
                    .bind(second_instance_id)
                    .fetch_one(&pool)
                    .await
                    .expect("read restarted persistent-service revision");
                    assert_eq!(
                        second_revision_id, service_fixture.revision_id,
                        "graceful restart must restore the same immutable service revision"
                    );
                    let second_resource_paths = {
                        let vm_id = format!("gateway-service-{second_instance_id}");
                        let provider_runtime_root = PathBuf::from(
                            env::var("HEPHAESTUS_LIBKRUN_RUNTIME_ROOT")
                                .expect("libkrun runtime root for restart cleanup assertion"),
                        );
                        let cgroup_root = PathBuf::from(
                            env::var("HEPHAESTUS_LIBKRUN_CGROUP_ROOT")
                                .expect("libkrun cgroup root for restart cleanup assertion"),
                        );
                        (
                            provider_runtime_root.join(&vm_id),
                            cgroup_root.join(&vm_id),
                            root.join("run-runtime")
                                .join("gateway-services")
                                .join(second_instance_id.to_string()),
                        )
                    };
                    let (provider_runtime, cgroup, materializer) = &second_resource_paths;
                    assert!(
                        provider_runtime.is_dir(),
                        "restarted service VM runtime exists before shutdown"
                    );
                    assert!(
                        cgroup.is_dir(),
                        "restarted service VM cgroup exists before shutdown"
                    );
                    assert!(
                        materializer.is_dir(),
                        "restarted service materializer tree exists before shutdown"
                    );
                    let restarted_admin_url = env::var("HEPHAESTUS_CADDY_TEST_ADMIN_URL")
                        .expect("joined Caddy admin URL");
                    let restarted_config =
                        wait_for_caddy_configuration(&restarted_admin_url, "/gateway/service")
                            .await;
                    assert!(
                        restarted_config.contains("/gateway/service"),
                        "Caddy must restore the persistent service route after daemon restart: {restarted_config}"
                    );
                    let second_service_proof = exercise_gateway_service_requests(
                        &env::var("HEPHAESTUS_CADDY_TEST_PUBLIC_URL")
                            .expect("joined Caddy public URL after restart"),
                    )
                    .await;
                    service_startup_id_before_crash = Some(second_service_proof.startup_id.clone());
                    assert_ne!(
                        first_service_proof.startup_id, second_service_proof.startup_id,
                        "restart must replace the guest startup identity"
                    );
                    println!(
                        "persistent-service-restart old_instance={first_instance_id} new_instance={second_instance_id} old_startup_id={} new_startup_id={} old_pid={} new_pid={}",
                        first_service_proof.startup_id,
                        second_service_proof.startup_id,
                        first_service_proof.pid,
                        second_service_proof.pid,
                    );
                    service_instance_id = Some(second_instance_id);
                    service_resource_paths = Some(second_resource_paths);
                }
            }
            let public_url =
                env::var("HEPHAESTUS_CADDY_TEST_PUBLIC_URL").expect("joined Caddy public URL");
            if gateway_service_e2e {
                let service_fixture = gateway_service_fixture
                    .as_ref()
                    .expect("persistent service fixture before crash proof");
                let crashed_instance_id =
                    service_instance_id.expect("persistent service instance before crash proof");
                let crashed_startup_id = service_startup_id_before_crash
                    .as_deref()
                    .expect("persistent service startup identity before crash");
                let crashed_resource_paths = service_resource_paths
                    .as_ref()
                    .expect("persistent service paths before crash proof");
                exercise_gateway_service_crash(&public_url).await;
                let replacement_instance_id = wait_for_gateway_service_crash_replacement(
                    &pool,
                    service_fixture,
                    crashed_instance_id,
                )
                .await;
                assert_ne!(
                    crashed_instance_id, replacement_instance_id,
                    "guest crash must create a replacement service instance"
                );
                let crash_evidence: (String, String, i32, Option<i32>) = sqlx::query_as(
                    "SELECT state, failure_code, exit_code, exit_signal
                       FROM gateway_service_instances
                      WHERE id = $1 AND gateway_id = $2 AND revision_id = $3",
                )
                .bind(crashed_instance_id)
                .bind(service_fixture.gateway_id)
                .bind(service_fixture.revision_id)
                .fetch_one(&pool)
                .await
                .expect("read durable guest crash evidence");
                assert_eq!(
                    crash_evidence,
                    (
                        String::from("cleaned"),
                        String::from("unexpected_exit"),
                        42,
                        None,
                    )
                );
                let (provider_runtime, cgroup, materializer) = crashed_resource_paths;
                assert!(!provider_runtime.exists());
                assert!(!cgroup.exists());
                assert!(!materializer.exists());
                let replacement_revision_id: uuid::Uuid = sqlx::query_scalar(
                    "SELECT revision_id FROM gateway_service_instances WHERE id = $1",
                )
                .bind(replacement_instance_id)
                .fetch_one(&pool)
                .await
                .expect("read replacement service revision after crash");
                assert_eq!(replacement_revision_id, service_fixture.revision_id);
                let replacement_resource_paths = {
                    let vm_id = format!("gateway-service-{replacement_instance_id}");
                    let provider_runtime_root = PathBuf::from(
                        env::var("HEPHAESTUS_LIBKRUN_RUNTIME_ROOT")
                            .expect("libkrun runtime root for crash cleanup assertion"),
                    );
                    let cgroup_root = PathBuf::from(
                        env::var("HEPHAESTUS_LIBKRUN_CGROUP_ROOT")
                            .expect("libkrun cgroup root for crash cleanup assertion"),
                    );
                    (
                        provider_runtime_root.join(&vm_id),
                        cgroup_root.join(&vm_id),
                        root.join("run-runtime")
                            .join("gateway-services")
                            .join(replacement_instance_id.to_string()),
                    )
                };
                let (provider_runtime, cgroup, materializer) = &replacement_resource_paths;
                assert!(provider_runtime.is_dir());
                assert!(cgroup.is_dir());
                assert!(materializer.is_dir());
                let replacement_proof = exercise_gateway_service_requests(&public_url).await;
                assert_ne!(
                    crashed_startup_id, replacement_proof.startup_id,
                    "guest crash must replace the startup identity"
                );
                println!(
                    "persistent-service-crash old_instance={crashed_instance_id} replacement_instance={replacement_instance_id} exit_code=42 replacement_startup_id={} replacement_pid={}",
                    replacement_proof.startup_id, replacement_proof.pid,
                );
                service_instance_id = Some(replacement_instance_id);
                service_resource_paths = Some(replacement_resource_paths);
            }
            let client = reqwest::Client::new();
            let first = client
                .post(format!("{public_url}/gateway/brokered?mode=real"))
                .header("x-webhook-secret", BROKERED_E2E_SENTINEL)
                .body("gateway-brokered-request-a")
                .send();
            let second = client
                .post(format!("{public_url}/gateway/brokered?mode=real"))
                .header("x-webhook-secret", BROKERED_E2E_SENTINEL)
                .body("gateway-brokered-request-b")
                .send();
            let (response, distinct) = tokio::join!(first, second);
            let response = response.expect("first concurrent Caddy gateway request");
            let distinct = distinct.expect("second concurrent Caddy gateway request");
            if response.status() != reqwest::StatusCode::CREATED
                || distinct.status() != reqwest::StatusCode::CREATED
            {
                let gateway_diagnostics: (i64, i64, i64, i64, i64, Vec<String>, Vec<String>) =
                    sqlx::query_as(
                        "SELECT
                         (SELECT count(*) FROM gateway_invocations),
                         (SELECT count(*) FROM gateway_authorization_snapshots),
                         (SELECT count(*) FROM gateway_authorization_snapshot_bindings),
                         (SELECT count(*) FROM gateway_runtime_authority_sessions),
                         (SELECT count(*) FROM gateway_mailbox_publications),
                         COALESCE((SELECT array_agg(status ORDER BY id)
                                   FROM gateway_runtime_authority_sessions), ARRAY[]::text[]),
                         COALESCE((SELECT array_agg(outcome ORDER BY id)
                                   FROM gateway_invocations), ARRAY[]::text[])",
                    )
                    .fetch_one(&pool)
                    .await
                    .expect("load gateway failure diagnostics");
                panic!(
                    "joined gateway requests failed: first={}, second={}, invocations={}, snapshots={}, snapshot_bindings={}, sessions={}, publications={}, session_statuses={:?}, invocation_outcomes={:?}",
                    response.status(),
                    distinct.status(),
                    gateway_diagnostics.0,
                    gateway_diagnostics.1,
                    gateway_diagnostics.2,
                    gateway_diagnostics.3,
                    gateway_diagnostics.4,
                    gateway_diagnostics.5,
                    gateway_diagnostics.6,
                );
            }
            assert_eq!(response.status(), reqwest::StatusCode::CREATED);
            assert_eq!(distinct.status(), reqwest::StatusCode::CREATED);
            assert_eq!(
                response.bytes().await.expect("gateway response body"),
                "gateway-brokered-header-ok"
            );
            assert_eq!(
                distinct
                    .bytes()
                    .await
                    .expect("distinct gateway response body"),
                "gateway-brokered-header-ok"
            );
            // A handler retry emits the same application-supplied key. It is
            // a fresh gateway invocation but must retain the first logical
            // mailbox event rather than delivering a second one.
            let retry = client
                .post(format!("{public_url}/gateway/brokered?mode=real"))
                .header("x-webhook-secret", BROKERED_E2E_SENTINEL)
                .body("gateway-brokered-request-a")
                .send()
                .await
                .expect("Caddy gateway retry");
            assert_eq!(retry.status(), reqwest::StatusCode::CREATED);
            assert_eq!(
                retry.bytes().await.expect("gateway retry response body"),
                "gateway-brokered-header-ok"
            );
            let evidence: (String, bool, bool) = sqlx::query_as(
                "SELECT invocation.outcome, session.acknowledged_at IS NOT NULL,
                        EXISTS (
                            SELECT 1 FROM gateway_secret_leases AS lease
                             WHERE lease.invocation_id = invocation.id
                        )
                   FROM gateway_invocations AS invocation
                   JOIN gateway_runtime_authority_sessions AS session
                     ON session.invocation_id = invocation.id
                  WHERE invocation.request_id IS NOT NULL
                  ORDER BY invocation.accepted_at DESC LIMIT 1",
            )
            .fetch_one(&pool)
            .await
            .expect("persisted joined gateway authority evidence");
            assert_eq!(evidence.0, "completed");
            assert!(evidence.1, "gateway VM acknowledged its runtime authority");
            assert!(
                evidence.2,
                "gateway invocation held its inbound secret lease"
            );
            let mailbox = gateway_edge
                .as_ref()
                .expect("joined gateway fixture")
                .1
                .mailbox_id;
            let (accepted_publications, duplicate_publications): (i64, i64) = sqlx::query_as(
                "SELECT count(*) FILTER (WHERE publication.outcome = 'accepted'),
                        count(*) FILTER (WHERE publication.outcome = 'duplicate')
                   FROM gateway_mailbox_publications AS publication
                   JOIN gateway_invocations AS invocation
                     ON invocation.id = publication.invocation_id
                  WHERE publication.mailbox_id = $1",
            )
            .bind(mailbox.as_uuid())
            .fetch_one(&pool)
            .await
            .expect("gateway mailbox publication provenance");
            assert_eq!((accepted_publications, duplicate_publications), (2, 1));
            let (event_count, wake_count, leaked_body): (i64, i64, bool) = sqlx::query_as(
                "SELECT
                     (SELECT count(*) FROM mailbox_events WHERE mailbox_id = $1),
                     (SELECT count(*) FROM outbox
                       WHERE subject = 'heph.mailbox.v1.wake'
                         AND id IN (SELECT id FROM mailbox_events WHERE mailbox_id = $1)),
                     EXISTS (
                       SELECT 1 FROM gateway_mailbox_publications
                        WHERE mailbox_id = $1
                          AND to_jsonb(gateway_mailbox_publications)::text
                              LIKE '%gateway-real-mailbox-body%'
                     )",
            )
            .bind(mailbox.as_uuid())
            .fetch_one(&pool)
            .await
            .expect("gateway mailbox acceptance evidence");
            assert_eq!((event_count, wake_count), (2, 2));
            assert!(!leaked_body, "gateway publication provenance is value-free");

            // This waits for the event emitted by the real libkrun guest to
            // cross the transactional outbox and JetStream dispatcher into a
            // separate real target-agent run. The target command rejects an
            // altered route or body before it can succeed.
            let delivery = tokio::time::timeout(Duration::from_secs(60), async {
                loop {
                    let row = sqlx::query_as::<_, (i64, i64)>(
                        "SELECT count(DISTINCT delivery.event_id)
                                  FILTER (WHERE delivery.disposition = 'delivered'),
                                count(*) FILTER (
                                    WHERE run.state = 'cleaned_up'
                                      AND run.outcome = 'succeeded'
                                )
                           FROM mailbox_deliveries AS delivery
                           JOIN mailbox_delivery_attempts AS attempt
                             ON attempt.event_id = delivery.event_id
                           JOIN runs AS run ON run.id = attempt.run_id
                          WHERE delivery.mailbox_id = $1",
                    )
                    .bind(mailbox.as_uuid())
                    .fetch_one(&pool)
                    .await
                    .expect("gateway mailbox delivery evidence");
                    if row == (2, 2) {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }
            })
            .await;
            if delivery.is_err() {
                let failures: Vec<(String, String, Option<String>, Option<String>)> =
                    sqlx::query_as(
                        "SELECT delivery.disposition, run.state, run.outcome, run.failure
                     FROM mailbox_delivery_attempts AS attempt
                     JOIN mailbox_deliveries AS delivery ON delivery.event_id = attempt.event_id
                     JOIN runs AS run ON run.id = attempt.run_id
                     WHERE delivery.mailbox_id = $1 ORDER BY run.id",
                    )
                    .bind(mailbox.as_uuid())
                    .fetch_all(&pool)
                    .await
                    .expect("gateway delivery failures");
                panic!("real gateway publications reach target-agent dispatch: {failures:?}");
            }

            // New invocations are denied after the exact bound grant is
            // revoked. This runs after the already accepted events settled so
            // it proves live authorization without perturbing their delivery.
            let fixture = &gateway_edge.as_ref().expect("joined gateway fixture").1;
            sqlx::query(
                "UPDATE gateway_mailbox_binding_grants
                    SET status = 'revoked', revoked_at = now(), revoked_by = $2
                  WHERE id = $1",
            )
            .bind(fixture.grant_id)
            .bind(user_id.as_uuid())
            .execute(&pool)
            .await
            .expect("revoke bound gateway mailbox grant");
            let denied = client
                .post(format!("{public_url}/gateway/brokered?mode=real"))
                .header("x-webhook-secret", BROKERED_E2E_SENTINEL)
                .body("gateway-brokered-request-denied")
                .send()
                .await
                .expect("revoked Caddy gateway request");
            assert_eq!(denied.status(), reqwest::StatusCode::BAD_GATEWAY);
            let accepted_after_revoke: i64 =
                sqlx::query_scalar("SELECT count(*) FROM mailbox_events WHERE mailbox_id = $1")
                    .bind(mailbox.as_uuid())
                    .fetch_one(&pool)
                    .await
                    .expect("revoked grant does not add mailbox events");
            assert_eq!(accepted_after_revoke, 2);
        }
        if let Some(fixture) = brokered_fixture.take() {
            fixture.upstream.assert_substituted_request().await;
        }
        running.shutdown().await.expect("graceful daemon shutdown");
        if let (
            Some(service_fixture),
            Some(instance_id),
            Some((provider_runtime, cgroup, materializer)),
        ) = (
            gateway_service_fixture.as_ref(),
            service_instance_id,
            service_resource_paths,
        ) {
            let state: Option<String> = sqlx::query_scalar(
                "SELECT state FROM gateway_service_instances
                  WHERE id = $1 AND gateway_id = $2 AND revision_id = $3",
            )
            .bind(instance_id)
            .bind(service_fixture.gateway_id)
            .bind(service_fixture.revision_id)
            .fetch_optional(&pool)
            .await
            .expect("read cleaned persistent-service instance");
            assert_eq!(state.as_deref(), Some("cleaned"));
            assert!(!provider_runtime.exists());
            assert!(!cgroup.exists());
            assert!(!materializer.exists());
        }
        if gateway_service_e2e {
            println!("persistent-service-e2e=passed");
        }
        cleanup_streams(&nats_url).await;
        return;
    }

    let (result_ref, result_commit): (String, String) = sqlx::query_as(
        "SELECT result_ref, result_commit
               FROM run_results WHERE run_id = $1 AND state = 'completed'",
    )
    .bind(run_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("completed result");
    let bare = root
        .join("repositories")
        .join(format!("{}.git", repository.id));
    assert_eq!(
        git_output_bare(&bare, &["rev-parse", &result_ref]).await,
        result_commit
    );
    assert_eq!(
        git_output_bare(&bare, &["rev-parse", &format!("{result_commit}^")]).await,
        input_commit
    );
    assert_eq!(
        git_output_bare(&bare, &["show", &format!("{result_commit}:input.txt")]).await,
        "agent edit"
    );

    let permissions: Vec<String> = sqlx::query_scalar(
        "SELECT permission FROM authorization_audit_events
           WHERE actor_id = $1
           ORDER BY permission",
    )
    .bind(user_id.as_uuid())
    .fetch_all(&pool)
    .await
    .expect("authorization audit");
    assert!(permissions.iter().any(|value| value == "can_write"));
    assert!(permissions.iter().any(|value| value == "can_execute"));
    assert!(permissions.iter().any(|value| value == "can_use"));
    let mapped_profile: bool = sqlx::query_scalar(
        "SELECT EXISTS(
              SELECT 1 FROM user_profiles
              WHERE user_id = $1
                AND validated_claims->>'sub' = 'golden-subject'
           )",
    )
    .bind(user_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("OIDC profile mapping");
    assert!(mapped_profile);

    // A durable mailbox journey reuses the production daemon, PostgreSQL
    // outbox, JetStream worker, run gate, fenced state volume, runtime
    // authority and configured VM backend. With `HEPHAESTUS_APP_LIBKRUN_E2E`
    // this is an actual libkrun guest, not a provider double.
    // Keep the fixture behind its gate during daemon startup. Startup/update
    // reconciliation must not acquire this state volume before this test's
    // mailbox command reaches the dispatch-time gate recheck.
    sqlx::query("UPDATE agent_instances SET run_gate_open = true WHERE id = $1")
        .bind(seeded_instance.instance)
        .execute(&pool)
        .await
        .expect("open mailbox proof run gate");
    let mailbox_repository = PostgresMailboxRepository::new(pool.clone());
    let mailbox_id = MailboxId::new();
    mailbox_repository
        .ensure_mailbox(
            project.id.as_uuid(),
            mailbox_id,
            runtime_types::AgentInstanceId::from_uuid(seeded_instance.instance),
        )
        .await
        .expect("create durable golden mailbox");
    let mailbox_body = b"golden-real-mailbox-body";
    let mailbox_event = MailboxEvent {
        id: mailbox_domain::MailboxEventId::new(),
        mailbox_id,
        instance_id: runtime_types::AgentInstanceId::from_uuid(seeded_instance.instance),
        producer_id: ProducerId::parse("golden-daemon-e2e").expect("producer"),
        deduplication_key: DeduplicationKey::parse("golden-mailbox-1").expect("deduplication"),
        envelope: MailboxEnvelope::new(
            EnvelopeMethod::parse("POST").expect("method"),
            EnvelopeRoute::parse("/mailbox/golden-proof").expect("route"),
            BTreeMap::new(),
            ContentMetadata::new(
                BodyReference::new(
                    BodyReferenceId::new(),
                    u32::try_from(mailbox_body.len()).expect("mailbox body length"),
                    Sha256::digest(mailbox_body).into(),
                )
                .expect("body reference"),
                Some(String::from("application/octet-stream")),
                Some(String::from("identity")),
            )
            .expect("content metadata"),
            OffsetDateTime::now_utc(),
            None,
        )
        .expect("mailbox envelope"),
    };
    let accepted = mailbox_repository
        .accept(
            project.id.as_uuid(),
            &mailbox_event,
            mailbox_body,
            u32::try_from(mailbox_body.len()).expect("mailbox body length"),
        )
        .await
        .expect("accept mailbox event through PostgreSQL");
    let mailbox_completion = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let row = sqlx::query_as::<
                _,
                (
                    String,
                    uuid::Uuid,
                    String,
                    Option<String>,
                    Option<uuid::Uuid>,
                    Option<uuid::Uuid>,
                    Option<i64>,
                ),
            >(
                "SELECT delivery.disposition, attempt.run_id, run.state, run.outcome,
                        attempt.authorization_snapshot_id, attempt.lease_id,
                        attempt.lease_fencing_token
                 FROM mailbox_deliveries AS delivery
                 JOIN mailbox_delivery_attempts AS attempt ON attempt.event_id = delivery.event_id
                 JOIN runs AS run ON run.id = attempt.run_id
                 WHERE delivery.event_id = $1
                 ORDER BY attempt.attempt_number DESC LIMIT 1",
            )
            .bind(accepted.event_id.as_uuid())
            .fetch_optional(&pool)
            .await
            .expect("load composed mailbox delivery");
            if let Some((disposition, run_id, state, outcome, snapshot, lease, fence)) = row {
                if disposition == "delivered" {
                    assert_eq!(state, "cleaned_up");
                    assert_eq!(outcome.as_deref(), Some("succeeded"));
                    assert!(snapshot.is_some(), "runtime authority snapshot is durable");
                    assert!(
                        lease.is_some() && fence.is_some(),
                        "fenced state lease is durable"
                    );
                    break run_id;
                }
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await;
    if mailbox_completion.is_err() {
        let evidence: Vec<MailboxTimeoutEvidence> = sqlx::query_as(
            "SELECT attempt.attempt_number, attempt.run_id, delivery.disposition,
                        attempt.state, run.state, run.outcome, run.failure
                   FROM mailbox_deliveries AS delivery
                   JOIN mailbox_delivery_attempts AS attempt ON attempt.event_id = delivery.event_id
                   JOIN runs AS run ON run.id = attempt.run_id
                  WHERE delivery.event_id = $1
                  ORDER BY attempt.attempt_number",
        )
        .bind(accepted.event_id.as_uuid())
        .fetch_all(&pool)
        .await
        .expect("load mailbox timeout evidence");
        eprintln!("golden mailbox timeout: evidence={evidence:?}");
    }
    let mailbox_run_id =
        mailbox_completion.expect("mailbox delivery reaches cleaned-up guest result");
    if let Some(fixture) = brokered_fixture.take() {
        fixture.upstream.assert_substituted_request().await;
    }
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT state_access_outcome FROM mailbox_delivery_attempts WHERE run_id = $1"
        )
        .bind(mailbox_run_id)
        .fetch_one(&pool)
        .await
        .expect("mailbox state access evidence"),
        "completed_access"
    );

    running.shutdown().await.expect("graceful daemon shutdown");
    let (signals, unpublished_signals): (i64, i64) = sqlx::query_as(
        "SELECT count(*) FILTER (WHERE message_class = 'internal_signal'),
                  count(*) FILTER (
                      WHERE message_class = 'internal_signal' AND published_at IS NULL
                  )
           FROM outbox",
    )
    .fetch_one(&pool)
    .await
    .expect("internal signal outbox census");
    assert_eq!(
        signals, 0,
        "legacy informational signals must not be emitted"
    );
    assert_eq!(
        unpublished_signals, 0,
        "legacy informational signals must never remain pending"
    );
    cleanup_streams(&nats_url).await;
    }).await;
    finish_isolated_golden(
        isolated_database,
        pool,
        &database_url,
        &parent_database_url,
        parent_before,
    )
    .await;
}

async fn wait_for_caddy_configuration(admin_url: &str, required_route: &str) -> String {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .expect("bounded Caddy configuration client");
    let mut last_configuration = String::new();
    for _ in 0..40 {
        last_configuration = client
            .get(format!("{admin_url}/config/"))
            .send()
            .await
            .expect("load applied Caddy configuration")
            .error_for_status()
            .expect("Caddy configuration request succeeds")
            .text()
            .await
            .expect("read applied Caddy configuration");
        if last_configuration.contains(required_route) {
            return last_configuration;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    panic!(
        "Caddy did not contain {required_route} after bounded reconciliation: {last_configuration}"
    );
}

async fn seed_golden_browser_session(
    pool: &sqlx::PgPool,
    user_id: UserId,
    issuer: &str,
    subject: &str,
) -> BrowserSessionSid {
    let sid = BrowserSessionSid::new();
    let verified = AuthenticatedIdentity::new(
        user_id,
        issuer,
        subject,
        serde_json::Value::Null,
        RequestId::new(),
    );
    sqlx::query(
        "INSERT INTO human_browser_sessions
            (id, sid_digest, creation_idempotency_id, creation_request_id,
             identity_binding_digest, user_id, issued_at, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, now(), now() + interval '12 hours')",
    )
    .bind(uuid::Uuid::new_v4())
    .bind(browser_session_sid_digest(sid).as_bytes().to_vec())
    .bind(uuid::Uuid::new_v4())
    .bind(uuid::Uuid::new_v4())
    .bind(
        browser_session_identity_binding_digest(&verified)
            .as_bytes()
            .to_vec(),
    )
    .bind(user_id.as_uuid())
    .execute(pool)
    .await
    .expect("seed active golden browser session");
    sid
}

fn signed_token(lifetime: Duration) -> String {
    let now = OffsetDateTime::now_utc().unix_timestamp();
    let lifetime_seconds = i64::try_from(lifetime.as_secs()).expect("bounded token lifetime");
    let issuer = golden_issuer();
    encode(
        &Header::new(Algorithm::HS256),
        &serde_json::json!({
            "iss": issuer,
            "sub": "golden-subject",
            "aud": AUDIENCE,
            "iat": now,
            "exp": now + lifetime_seconds,
            "email": "golden@example.invalid",
            "email_verified": true
        }),
        &EncodingKey::from_secret(SIGNING_SECRET),
    )
    .expect("sign golden bearer token")
}

fn installed_ui_fixture_enabled() -> bool {
    env::var("HEPHAESTUS_COOKING_INSTALLED_UI_FIXTURE").as_deref() == Ok("1")
}

fn installed_ui_platform_origin() -> String {
    let public =
        Url::parse(&env::var("HEPHAESTUS_CADDY_TEST_PUBLIC_URL").expect("joined Caddy public URL"))
            .expect("joined Caddy public URL");
    let port = public.port().expect("joined Caddy public port");
    format!("https://platform.localhost:{port}")
}

fn installed_ui_namespace() -> String {
    env::var("HEPHAESTUS_UI_NAMESPACE").unwrap_or_else(|_| String::from("ui.platform.localhost"))
}

fn reserve_installed_ui_listener() -> std::net::SocketAddr {
    std::net::TcpListener::bind("127.0.0.1:0")
        .expect("reserve installed UI origin listener")
        .local_addr()
        .expect("installed UI origin listener address")
}

fn installed_ui_origin_config(listener: std::net::SocketAddr) -> UiOriginConfig {
    let public =
        Url::parse(&env::var("HEPHAESTUS_CADDY_TEST_PUBLIC_URL").expect("joined Caddy public URL"))
            .expect("joined Caddy public URL");
    let public_port = UiPublicPort::parse(public.port().expect("joined Caddy public port"))
        .expect("joined Caddy public port is valid");
    UiOriginConfig::new(
        UiNamespace::parse(installed_ui_namespace()).expect("installed UI namespace"),
        public_port,
        installed_ui_platform_origin(),
    )
    .expect("installed UI origin configuration")
    .with_listener(listener)
}

fn caddy_configuration(admin_url: &str) -> Vec<u8> {
    let admin = reqwest::Url::parse(admin_url).expect("Caddy admin URL");
    let admin_listen = format!(
        "{}:{}",
        admin.host_str().expect("Caddy admin host"),
        admin.port().expect("Caddy admin port")
    );
    let mut routes = vec![
        serde_json::json!({
            "match": [{ "path": ["/platform/*"] }],
            "handle": [{ "handler": "static_response", "body": "platform-owned" }]
        }),
        serde_json::json!({
            "group": "hephaestus.gateway",
            "handle": [{ "handler": "subroute", "routes": [] }]
        }),
        serde_json::json!({ "handle": [{ "handler": "static_response", "status_code": 404 }] }),
    ];
    if installed_ui_fixture_enabled() {
        let web_port =
            env::var("HEPHAESTUS_E2E_EXTERNAL_WEB_PORT").unwrap_or_else(|_| "4000".into());
        let platform_host = Url::parse(&installed_ui_platform_origin())
            .expect("installed UI platform origin")
            .host_str()
            .expect("installed UI platform host")
            .to_owned();
        routes.insert(
            0,
            serde_json::json!({
                "group": "hephaestus.ui",
                "handle": [{ "handler": "subroute", "routes": [] }]
            }),
        );
        routes.insert(
            1,
            serde_json::json!({
                "match": [{ "host": [platform_host] }],
                "handle": [{
                    "handler": "reverse_proxy",
                    "upstreams": [{ "dial": format!("127.0.0.1:{web_port}") }]
                }],
                "terminal": true
            }),
        );
    }
    let mut configuration = serde_json::json!({
        "admin": { "listen": admin_listen },
        "apps": { "http": { "servers": { "shared": {
            "listen": [env::var("HEPHAESTUS_CADDY_TEST_LISTEN").expect("joined Caddy listen address")],
            "routes": routes
        } } } }
    });
    if env::var("HEPHAESTUS_CADDY_TEST_TLS").as_deref() == Ok("1") {
        let server = configuration
            .pointer_mut("/apps/http/servers/shared")
            .expect("shared Caddy server configuration");
        server["automatic_https"] = serde_json::json!({ "disable_redirects": true });
        server["tls_connection_policies"] = serde_json::json!([{}]);
        let subjects = if installed_ui_fixture_enabled() {
            serde_json::json!([
                "127.0.0.1",
                Url::parse(&installed_ui_platform_origin())
                    .expect("installed UI platform origin")
                    .host_str()
                    .expect("installed UI platform host")
                    .to_owned(),
                format!("*.{}", installed_ui_namespace())
            ])
        } else {
            serde_json::json!(["127.0.0.1"])
        };
        configuration["apps"]["tls"] = serde_json::json!({
            "certificates": { "automate": subjects },
            "automation": {
                "policies": [{
                    "subjects": subjects,
                    "issuers": [{ "module": "internal" }]
                }]
            }
        });
    }
    serde_json::to_vec(&configuration).expect("serialize Caddy configuration template")
}

const fn agent_config() -> &'static str {
    r#"
  version = 2
  [agent]
  name = "Golden Agent"
  key = "golden-agent"
  [build]
  command = "/bin/sh"
  arguments = ["-c", "mkdir -p /workspace/output/bin && cp /workspace/source/golden-agent.sh /workspace/output/bin/golden && chmod 0555 /workspace/output/bin/golden"]
  working_directory = "/workspace/source"
  image = { key = "golden-root" }
  triggers = ["refs/heads/main"]
  [build.resources]
  vcpus = 1
  memory_mib = 512
  [build.network]
  profile = "disabled"
  [[build.artifacts]]
  path = "bin/golden"
  kind = "executable"
  [guest]
  image = { key = "golden-root" }
  command = "bin/golden"
  arguments = []
  working_directory = "bin"
  [resources]
  vcpus = 1
  memory_mib = 512
  [workspace]
  mount = true
  path = "/workspace/repo"
  read_only = true
  [state_volume]
  enabled = true
  [network]
  profile = "disabled"
  [triggers]
  push = false
  refs = ["refs/heads/main"]
  [update_hook]
  command = "bin/golden"
  arguments = []
  timeout_seconds = 60
  [update_hook.resources]
  vcpus = 1
  memory_mib = 512
  "#
}

struct ForgeSourceFixture {
    input_commit: String,
    run_id: uuid::Uuid,
}

#[derive(Clone, Copy)]
enum ForgeSourceContent {
    GoldenAgent,
    CookingBlog,
}

// Keep the exact Git, request, result, and proposal assertions together so
// the browser retry fixture cannot accidentally lose one provenance boundary.
#[allow(
    clippy::cognitive_complexity,
    clippy::too_many_arguments,
    clippy::too_many_lines
)]
async fn create_forge_source_run(
    pool: &sqlx::PgPool,
    running: &hephaestus_app::RunningHephaestus,
    root: &Path,
    repository_id: uuid::Uuid,
    instance_id: uuid::Uuid,
    token: &str,
    libkrun_e2e: bool,
    require_review_proposal: bool,
    source_content: ForgeSourceContent,
) -> ForgeSourceFixture {
    let source = root.join("source");
    tokio::fs::create_dir(&source)
        .await
        .expect("source repository");
    git(&source, &["init", "--initial-branch=main"]).await;
    git(&source, &["config", "user.name", "Golden Test"]).await;
    git(&source, &["config", "user.email", "golden@example.invalid"]).await;
    tokio::fs::write(source.join("agent.toml"), agent_config())
        .await
        .expect("provider-neutral agent.toml");
    tokio::fs::write(source.join("golden-agent.sh"), GOLDEN_AGENT)
        .await
        .expect("golden agent executable source");
    tokio::fs::write(source.join("input.txt"), "accepted\n")
        .await
        .expect("input file");
    if matches!(source_content, ForgeSourceContent::CookingBlog) {
        cooking::copy_blog(&source).await;
    }
    tokio::fs::create_dir(source.join("reports"))
        .await
        .expect("reports directory");
    tokio::fs::write(source.join("reports/result.txt"), "initial\n")
        .await
        .expect("initial report");
    git(&source, &["add", "."]).await;
    git(&source, &["commit", "-m", "golden agent"]).await;
    if matches!(source_content, ForgeSourceContent::GoldenAgent) {
        let committed_agent = git_output(&source, &["show", "HEAD:agent.toml"]).await;
        assert_eq!(
            committed_agent,
            agent_config().trim(),
            "forge retry source must retain the provider-neutral agent manifest"
        );
        assert!(
            !source.join("heph.images.toml").exists(),
            "forge retry source must not select a repository image"
        );
        assert!(
            committed_agent.contains("image = { key = \"golden-root\" }"),
            "forge retry source must select the registered golden-root image"
        );
    }
    let input_commit = git_output(&source, &["rev-parse", "HEAD"]).await;
    let remote = format!("http://{}/{}", running.http_addr(), repository_id);
    git(&source, &["remote", "add", "origin", &remote]).await;
    sqlx::query("UPDATE agent_instances SET run_gate_open = true WHERE id = $1")
        .bind(instance_id)
        .execute(pool)
        .await
        .expect("open golden push run gate");
    authenticated_git(&source, token, &["push", "origin", "HEAD:refs/heads/main"]).await;
    let run_id: uuid::Uuid = sqlx::query_scalar(
        "SELECT request.run_id
           FROM run_requests request
          WHERE request.repository_id = $1
            AND request.instance_id = $2
            AND request.commit_sha = $3
            AND request.git_ref = 'refs/heads/main'
            AND request.retry_of_run_id IS NULL
            AND request.request_kind = 'instance_normal'
          ORDER BY request.created_at DESC, request.id DESC
          LIMIT 1",
    )
    .bind(repository_id)
    .bind(instance_id)
    .bind(&input_commit)
    .fetch_one(pool)
    .await
    .expect("durable forge run request");
    let runtime_run_id = runtime_types::RunId::from_uuid(run_id);
    let result_wait = running
        .wait_for_run_event(
            runtime_run_id,
            RunEventKind::ResultCompleted,
            Duration::from_secs(if libkrun_e2e { 60 } else { 10 }),
        )
        .await;
    if result_wait.is_err() {
        diagnose_golden_timeout(pool, repository_id, runtime_run_id).await;
    }
    result_wait.expect("persisted forge source result completion");
    let result_commit: Option<String> = sqlx::query_scalar(
        "SELECT result_commit FROM run_results
          WHERE run_id = $1 AND state = 'completed'",
    )
    .bind(run_id)
    .fetch_one(pool)
    .await
    .expect("completed forge source result");
    if require_review_proposal {
        assert!(
            result_commit.is_some(),
            "forge source run must produce a result commit for review retry"
        );
        let proposal_state: String = sqlx::query_scalar(
            "SELECT state FROM review_proposals
              WHERE run_id = $1",
        )
        .bind(run_id)
        .fetch_one(pool)
        .await
        .expect("forge source review proposal");
        assert!(
            matches!(proposal_state.as_str(), "open" | "approval_requested"),
            "forge source proposal remains reviewable"
        );
    }
    ForgeSourceFixture {
        input_commit,
        run_id,
    }
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
async fn seed_reusable_instance(
    pool: &sqlx::PgPool,
    actor: UserId,
    project_id: uuid::Uuid,
    repository_id: uuid::Uuid,
    artifact_root: &Path,
    brokered_https: bool,
    artifact_override: Option<&[u8]>,
    instance_name: &str,
) -> SeededInstance {
    let build_id = uuid::Uuid::new_v4();
    let family_id = uuid::Uuid::new_v4();
    let release_id = uuid::Uuid::new_v4();
    let release_agent_id = uuid::Uuid::new_v4();
    let instance_id = uuid::Uuid::new_v4();
    let revision_id = uuid::Uuid::new_v4();
    let attachment_id = uuid::Uuid::new_v4();
    let state_volume_id = uuid::Uuid::new_v4();
    let artifact_id = uuid::Uuid::new_v4();
    let storage_key = uuid::Uuid::new_v4();
    let cooking_artifact = cooking::agent_artifact();
    let artifact = artifact_override
        .or(cooking_artifact.as_deref())
        .unwrap_or(GOLDEN_AGENT.as_bytes());
    let release_configuration = serde_json::to_value(
        agent_config::parse(agent_config().as_bytes())
            .config
            .expect("golden reusable configuration should parse"),
    )
    .expect("serialize golden reusable configuration");
    tokio::fs::create_dir_all(artifact_root)
        .await
        .expect("release artifact root");
    let artifact_path = artifact_root.join(storage_key.simple().to_string());
    tokio::fs::write(&artifact_path, artifact)
        .await
        .expect("release artifact");
    let mut permissions = tokio::fs::metadata(&artifact_path)
        .await
        .expect("artifact metadata")
        .permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut permissions, 0o555);
    tokio::fs::set_permissions(&artifact_path, permissions)
        .await
        .expect("artifact mode");
    let artifact_hash: [u8; 32] = Sha256::digest(artifact).into();

    let secret_slot_schema = if artifact_override.is_some() {
        serde_json::json!([])
    } else if cooking::enabled() {
        cooking::secret_slots()
    } else if brokered_https {
        serde_json::json!([{
            "key": "model",
            "purpose": "Call a fixture HTTPS API",
            "required": true,
            "delivery_modes": ["brokered"],
            "phases": ["normal"],
            "destinations": ["api.example.test"]
        }])
    } else {
        serde_json::json!([])
    };
    sqlx::query(
        "INSERT INTO build_requests
           (id, repository_id, source_commit, source_ref,
            build_definition_hash, state, created_by, completed_at)
           VALUES ($1, $2, $3, 'refs/heads/main', $4, 'succeeded',
                   $5, now())",
    )
    .bind(build_id)
    .bind(repository_id)
    .bind("a".repeat(40))
    .bind([1_u8; 32].as_slice())
    .bind(actor.as_uuid())
    .execute(pool)
    .await
    .expect("seed reusable build");
    sqlx::query(
        "INSERT INTO agent_families (id, repository_id, agent_key)
           VALUES ($1, $2, 'golden-agent')",
    )
    .bind(family_id)
    .bind(repository_id)
    .execute(pool)
    .await
    .expect("seed reusable family");
    sqlx::query(
        "INSERT INTO releases
           (id, repository_id, version, source_commit, source_ref,
           build_request_id, build_definition_hash, configuration,
            configuration_hash, manifest_hash, state, published_at)
           VALUES ($1, $2, 'v1', $3, 'refs/heads/main', $4, $5,
                   $6, $7, $8, 'published', now())",
    )
    .bind(release_id)
    .bind(repository_id)
    .bind("a".repeat(40))
    .bind(build_id)
    .bind([1_u8; 32].as_slice())
    .bind(release_configuration)
    .bind([2_u8; 32].as_slice())
    .bind([3_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("seed reusable release");
    sqlx::query(
        "INSERT INTO release_artifacts
           (id, release_id, path, kind, mode, content_hash, size_bytes,
            media_type, storage_key)
           VALUES ($1, $2, 'bin/golden', 'executable', 365, $3, $4,
                   'application/octet-stream', $5)",
    )
    .bind(artifact_id)
    .bind(release_id)
    .bind(artifact_hash.as_slice())
    .bind(i64::try_from(artifact.len()).expect("artifact length"))
    .bind(storage_key)
    .execute(pool)
    .await
    .expect("seed reusable artifact");
    sqlx::query(
        "INSERT INTO release_agents
           (id, release_id, family_id, agent_key, display_name,
            runtime_contract, runtime_contract_hash, parameter_schema,
            secret_slot_schema, requires_state, update_hook)
           VALUES ($1, $2, $3, 'golden-agent', 'Golden Agent', $4, $5,
                   '[]', $6, true, $7)",
    )
    .bind(release_agent_id)
    .bind(release_id)
    .bind(family_id)
    .bind(serde_json::json!({
        "executable": "bin/golden",
        "arguments": [],
        "working_directory": "bin",
        "image_reference": ROOT_IMAGE,
        "root_image_digest": ROOT_IMAGE,
        "requires_state": true,
        "policy_ceiling": {
            "vcpus": 1,
            "memory_mib": 512,
            "network": if brokered_https { "broker_only" } else { "disabled" }
        }
    }))
    .bind([4_u8; 32].as_slice())
    .bind(secret_slot_schema)
    .bind(serde_json::json!({
        "command": "bin/golden",
        "arguments": [],
        "timeout_seconds": 60,
        "resources": {"vcpus": 1, "memory_mib": 512}
    }))
    .execute(pool)
    .await
    .expect("seed reusable release agent");
    sqlx::query(
        "INSERT INTO agent_instances
           (id, project_id, family_id, name, state, run_gate_open, created_by)
           VALUES ($1, $2, $3, $4, 'active', false, $5)",
    )
    .bind(instance_id)
    .bind(project_id)
    .bind(family_id)
    .bind(instance_name)
    .bind(actor.as_uuid())
    .execute(pool)
    .await
    .expect("seed reusable instance");
    sqlx::query(
        "INSERT INTO agent_instance_state_volumes
           (id, instance_id, state, capacity_bytes)
           VALUES ($1, $2, 'uninitialized', $3)",
    )
    .bind(state_volume_id)
    .bind(instance_id)
    .bind(16_i64 * 1024 * 1024)
    .execute(pool)
    .await
    .expect("seed reusable instance state volume");
    sqlx::query("UPDATE agent_instances SET state_volume_id = $2 WHERE id = $1")
        .bind(instance_id)
        .bind(state_volume_id)
        .execute(pool)
        .await
        .expect("attach reusable instance state volume");
    sqlx::query(
        "INSERT INTO agent_instance_revisions
           (id, instance_id, release_agent_id, parameters, parameter_hash,
            secret_bindings, resource_selection, network_restriction,
            effective_runtime_policy, effective_policy_hash,
            platform_policy_version, runnable, diagnostics, created_by)
           VALUES ($1, $2, $3, $9, $4, '[]', $5, $6, $5, $7,
                   'platform/v1', true, '[]', $8)",
    )
    .bind(revision_id)
    .bind(instance_id)
    .bind(release_agent_id)
    .bind([5_u8; 32].as_slice())
    .bind(serde_json::json!({
        "vcpus": 1,
        "memory_mib": 512,
        "network": if brokered_https { "broker_only" } else { "disabled" }
    }))
    .bind(serde_json::json!({
        "network": if brokered_https { "broker_only" } else { "disabled" }
    }))
    .bind([6_u8; 32].as_slice())
    .bind(actor.as_uuid())
    .bind(cooking::parameters())
    .execute(pool)
    .await
    .expect("seed reusable revision");
    sqlx::query("UPDATE agent_instances SET active_revision_id = $2 WHERE id = $1")
        .bind(instance_id)
        .bind(revision_id)
        .execute(pool)
        .await
        .expect("activate reusable revision");
    sqlx::query(
        "INSERT INTO agent_attachments
           (id, instance_id, project_id, repository_id, ref_selector,
            trigger_policy, enabled, created_by)
           VALUES ($1, $2, $3, $4, 'refs/heads/main', 'push', true, $5)",
    )
    .bind(attachment_id)
    .bind(instance_id)
    .bind(project_id)
    .bind(repository_id)
    .bind(actor.as_uuid())
    .execute(pool)
    .await
    .expect("seed reusable attachment");
    SeededInstance {
        instance: instance_id,
        revision: revision_id,
        attachment: attachment_id,
        release: release_id,
        release_agent: release_agent_id,
    }
}

struct SeededInstance {
    instance: uuid::Uuid,
    revision: uuid::Uuid,
    attachment: uuid::Uuid,
    release: uuid::Uuid,
    release_agent: uuid::Uuid,
}

struct ForgeRetryFixture {
    repository_id: uuid::Uuid,
    instance: SeededInstance,
    source_run_id: uuid::Uuid,
}

/// Exact host-side identity retained by the joined Caddy/libkrun mailbox
/// proof. The released guest never receives this identifier.
struct GatewayGoldenFixture {
    mailbox_id: MailboxId,
    grant_id: uuid::Uuid,
}

/// Exact durable declaration used by the opt-in daemon-to-Caddy service proof.
/// The desired pointer is set by the fixture, while the active pointer remains
/// unset until the production supervisor has observed HTTP readiness.
struct GatewayServiceGoldenFixture {
    gateway_id: uuid::Uuid,
    revision_id: uuid::Uuid,
}

type GatewayServiceCrashEvidence = (String, Option<String>, Option<i32>, Option<i32>);

/// Adds an exact released, stateless gateway handler alongside the reusable
/// agent. The artifact delegates only to the guest integration checker, which
/// validates that the daemon replaced the inbound secret before VM delivery.
async fn seed_gateway_release_agent(
    pool: &sqlx::PgPool,
    instance: &SeededInstance,
    artifact_root: &Path,
) -> uuid::Uuid {
    let agent_id = uuid::Uuid::new_v4();
    let artifact_id = uuid::Uuid::new_v4();
    let storage_key = uuid::Uuid::new_v4();
    let cooking_artifact = cooking::gateway_artifact();
    let artifact = cooking_artifact
        .as_deref()
        .unwrap_or(GATEWAY_HANDLER.as_bytes());
    let artifact_path = artifact_root.join(storage_key.simple().to_string());
    tokio::fs::write(&artifact_path, artifact)
        .await
        .expect("gateway release artifact");
    let mut permissions = tokio::fs::metadata(&artifact_path)
        .await
        .expect("gateway artifact metadata")
        .permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut permissions, 0o555);
    tokio::fs::set_permissions(&artifact_path, permissions)
        .await
        .expect("gateway artifact mode");
    let artifact_hash: [u8; 32] = Sha256::digest(artifact).into();
    sqlx::query(
        "INSERT INTO release_artifacts
           (id, release_id, path, kind, mode, content_hash, size_bytes,
            media_type, storage_key)
         VALUES ($1, $2, 'bin/gateway', 'executable', 365, $3, $4,
                 'application/octet-stream', $5)",
    )
    .bind(artifact_id)
    .bind(instance.release)
    .bind(artifact_hash.as_slice())
    .bind(i64::try_from(artifact.len()).expect("gateway artifact length"))
    .bind(storage_key)
    .execute(pool)
    .await
    .expect("seed gateway release artifact");
    let family_id: uuid::Uuid =
        sqlx::query_scalar("SELECT family_id FROM release_agents WHERE id = $1")
            .bind(instance.release_agent)
            .fetch_one(pool)
            .await
            .expect("load reusable release family");
    sqlx::query(
        "INSERT INTO release_agents
           (id, release_id, family_id, agent_key, display_name,
            runtime_contract, runtime_contract_hash, parameter_schema,
            secret_slot_schema, requires_state, update_hook)
         VALUES ($1, $2, $3, 'golden-gateway', 'Golden gateway', $4, $5,
                 '[]', '[]', false, NULL)",
    )
    .bind(agent_id)
    .bind(instance.release)
    .bind(family_id)
    .bind(serde_json::json!({
        "executable": "bin/gateway",
        "arguments": [],
        "working_directory": "bin",
        "image_reference": ROOT_IMAGE,
        "requires_state": false,
        "policy_ceiling": {"vcpus": 1, "memory_mib": 512, "network": "disabled"}
    }))
    .bind([7_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("seed stateless gateway release agent");
    agent_id
}

/// Adds the long-lived integration-check service executable to the published
/// release. It shares the release with the stateless proof but uses a distinct
/// artifact path and release-agent key, so the two immutable contracts cannot
/// accidentally select one another.
async fn seed_gateway_service_release_agent(
    pool: &sqlx::PgPool,
    instance: &SeededInstance,
    artifact_root: &Path,
) -> uuid::Uuid {
    let agent_id = uuid::Uuid::new_v4();
    let artifact_id = uuid::Uuid::new_v4();
    let storage_key = uuid::Uuid::new_v4();
    let artifact = SERVICE_GATEWAY_HANDLER.as_bytes();
    tokio::fs::create_dir_all(artifact_root)
        .await
        .expect("service gateway release artifact root");
    let artifact_path = artifact_root.join(storage_key.simple().to_string());
    tokio::fs::write(&artifact_path, artifact)
        .await
        .expect("service gateway release artifact");
    let mut permissions = tokio::fs::metadata(&artifact_path)
        .await
        .expect("service gateway artifact metadata")
        .permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut permissions, 0o555);
    tokio::fs::set_permissions(&artifact_path, permissions)
        .await
        .expect("service gateway artifact mode");
    let artifact_hash: [u8; 32] = Sha256::digest(artifact).into();
    sqlx::query(
        "INSERT INTO release_artifacts
           (id, release_id, path, kind, mode, content_hash, size_bytes,
            media_type, storage_key)
         VALUES ($1, $2, 'bin/service', 'executable', 365, $3, $4,
                 'application/octet-stream', $5)",
    )
    .bind(artifact_id)
    .bind(instance.release)
    .bind(artifact_hash.as_slice())
    .bind(i64::try_from(artifact.len()).expect("service artifact length"))
    .bind(storage_key)
    .execute(pool)
    .await
    .expect("seed service gateway release artifact");
    let family_id: uuid::Uuid =
        sqlx::query_scalar("SELECT family_id FROM release_agents WHERE id = $1")
            .bind(instance.release_agent)
            .fetch_one(pool)
            .await
            .expect("load reusable release family for service");
    sqlx::query(
        "INSERT INTO release_agents
           (id, release_id, family_id, agent_key, display_name,
            runtime_contract, runtime_contract_hash, parameter_schema,
            secret_slot_schema, requires_state, update_hook)
         VALUES ($1, $2, $3, 'golden-service', 'Golden service', $4, $5,
                 '[]', '[]', false, NULL)",
    )
    .bind(agent_id)
    .bind(instance.release)
    .bind(family_id)
    .bind(serde_json::json!({
        "executable": "bin/service",
        "arguments": [],
        "working_directory": "bin",
        "image_reference": ROOT_IMAGE,
        "requires_state": false,
        "policy_ceiling": {"vcpus": 1, "memory_mib": 512, "network": "disabled"}
    }))
    .bind([11_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("seed service gateway release agent");
    agent_id
}

/// Seeds one public `http.service.v1` declaration without pre-starting it.
/// The daemon must claim the desired revision, reach readiness, and promote
/// the active pointer before public requests are attempted.
async fn seed_gateway_service_route(
    pool: &sqlx::PgPool,
    actor: UserId,
    project_id: uuid::Uuid,
    repository_id: uuid::Uuid,
    release_id: uuid::Uuid,
    release_agent_id: uuid::Uuid,
    capture_application_logs: bool,
) -> GatewayServiceGoldenFixture {
    let gateway_id = uuid::Uuid::new_v4();
    let revision_id = uuid::Uuid::new_v4();
    let identity_route_id = uuid::Uuid::new_v4();
    let service_route_id = uuid::Uuid::new_v4();
    sqlx::query(
        "INSERT INTO gateways
           (id, project_id, repository_id, name, lifecycle, created_by)
         VALUES ($1, $2, $3, 'golden-service', 'enabled', $4)",
    )
    .bind(gateway_id)
    .bind(project_id)
    .bind(repository_id)
    .bind(actor.as_uuid())
    .execute(pool)
    .await
    .expect("seed service gateway");
    sqlx::query(
        "INSERT INTO gateway_revisions
           (id, gateway_id, project_id, repository_id, release_id, release_agent_id,
            release_agent_key, handler_contract, exposure, parameters, secret_slots,
            mailbox_slots, normalized_hash, created_by, service_loopback_port,
            service_readiness_path, service_health_path, service_log_capture_mode)
         VALUES ($1, $2, $3, $4, $5, $6, 'golden-service', 'http.service.v1',
                 'public', '{}'::jsonb, '{}', '{}', $7, $8, 8080, '/readyz', '/healthz', $9)",
    )
    .bind(revision_id)
    .bind(gateway_id)
    .bind(project_id)
    .bind(repository_id)
    .bind(release_id)
    .bind(release_agent_id)
    .bind([12_u8; 32].as_slice())
    .bind(actor.as_uuid())
    .bind(if capture_application_logs {
        "application"
    } else {
        "disabled"
    })
    .execute(pool)
    .await
    .expect("seed service gateway revision");
    for (route_id, path) in [
        (service_route_id, "/service"),
        (identity_route_id, "/service/identity"),
        (uuid::Uuid::new_v4(), "/service/crash"),
        (uuid::Uuid::new_v4(), "/service/log"),
    ] {
        sqlx::query(
            "INSERT INTO gateway_routes
               (id, gateway_revision_id, gateway_id, project_id, path, methods)
             VALUES ($1, $2, $3, $4, $5, ARRAY['GET'])",
        )
        .bind(route_id)
        .bind(revision_id)
        .bind(gateway_id)
        .bind(project_id)
        .bind(path)
        .execute(pool)
        .await
        .expect("seed service gateway route");
    }
    sqlx::query("UPDATE gateways SET desired_service_revision_id = $2 WHERE id = $1")
        .bind(gateway_id)
        .bind(revision_id)
        .execute(pool)
        .await
        .expect("publish service gateway desired revision");
    GatewayServiceGoldenFixture {
        gateway_id,
        revision_id,
    }
}

/// Creates a second independently published service release while leaving the
/// existing desired pointer untouched.  The cutover proof advances that
/// pointer only after the first public invocation has been accepted.
// The SQL fixture deliberately mirrors the published-release rows as one
// bounded setup operation; splitting it would obscure the immutable IDs.
#[allow(clippy::too_many_lines)]
async fn seed_gateway_service_cutover_candidate(
    pool: &sqlx::PgPool,
    fixture: &GatewayServiceGoldenFixture,
    artifact_root: &Path,
    readiness_path: &str,
) -> GatewayServiceGoldenFixture {
    let (project_id, repository_id, source_agent_key, source_publication_actor_id, created_by): (
        uuid::Uuid,
        uuid::Uuid,
        String,
        Option<uuid::Uuid>,
        uuid::Uuid,
    ) = sqlx::query_as(
        "SELECT gateway.project_id, gateway.repository_id,
                revision.release_agent_key, release.publication_actor_id,
                gateway.created_by
           FROM gateways AS gateway
           JOIN gateway_revisions AS revision
             ON revision.id = $2 AND revision.gateway_id = gateway.id
           JOIN releases AS release
             ON release.id = revision.release_id
          WHERE gateway.id = $1",
    )
    .bind(fixture.gateway_id)
    .bind(fixture.revision_id)
    .fetch_one(pool)
    .await
    .expect("read published service source metadata");
    let publication_actor_id = source_publication_actor_id.unwrap_or(created_by);
    let authorized_actor: Option<uuid::Uuid> = sqlx::query_scalar(
        "SELECT member.user_id
           FROM gateways AS gateway
           JOIN projects AS project ON project.id = gateway.project_id
           JOIN organization_members AS member
             ON member.organization_id = project.organization_id
            AND member.user_id = $2
          WHERE gateway.id = $1
            AND member.role IN ('owner', 'admin')",
    )
    .bind(fixture.gateway_id)
    .bind(publication_actor_id)
    .fetch_optional(pool)
    .await
    .expect("verify cutover publication actor authorization");
    assert_eq!(
        authorized_actor,
        Some(publication_actor_id),
        "cutover publication actor must belong to the gateway project organization"
    );
    let release_id = uuid::Uuid::new_v4();
    let build_request_id = uuid::Uuid::new_v4();
    let family_id = uuid::Uuid::new_v4();
    let release_agent_id = uuid::Uuid::new_v4();
    let artifact_id = uuid::Uuid::new_v4();
    let storage_key = uuid::Uuid::new_v4();
    let revision_id = uuid::Uuid::new_v4();
    let source_commit = format!("{:040x}", release_id.as_u128());
    let mut normalized_hash = [0_u8; 32];
    normalized_hash[..16].copy_from_slice(release_id.as_bytes());
    normalized_hash[16..].copy_from_slice(release_id.as_bytes());

    sqlx::query(
        "INSERT INTO build_requests
            (id, repository_id, source_commit, source_ref,
             build_definition_hash, state, created_by)
         VALUES ($1, $2, $3, 'refs/heads/main', $4, 'succeeded', $5)",
    )
    .bind(build_request_id)
    .bind(repository_id)
    .bind(&source_commit)
    .bind([21_u8; 32].as_slice())
    .bind(publication_actor_id)
    .execute(pool)
    .await
    .expect("seed cutover build request");
    sqlx::query(
        "INSERT INTO releases
            (id, repository_id, version, source_commit, source_ref,
             build_request_id, build_definition_hash, configuration,
             configuration_hash, manifest_hash, state,
             publication_actor_id, published_at)
         VALUES ($1, $2, $3, $4, 'refs/heads/main', $5, $6, '{}',
                 $7, $8, 'published', $9, now())",
    )
    .bind(release_id)
    .bind(repository_id)
    .bind(format!("cutover-{}", release_id.simple()))
    .bind(&source_commit)
    .bind(build_request_id)
    .bind([21_u8; 32].as_slice())
    .bind([22_u8; 32].as_slice())
    .bind([23_u8; 32].as_slice())
    .bind(publication_actor_id)
    .execute(pool)
    .await
    .expect("publish cutover release");
    sqlx::query(
        "INSERT INTO agent_families (id, repository_id, agent_key)
         VALUES ($1, $2, $3)",
    )
    .bind(family_id)
    .bind(repository_id)
    .bind(format!("cutover-agent-{}", release_id.simple()))
    .execute(pool)
    .await
    .expect("seed cutover agent family");
    sqlx::query(
        "INSERT INTO release_agents
            (id, release_id, family_id, agent_key, display_name,
             runtime_contract, runtime_contract_hash, parameter_schema,
             secret_slot_schema, requires_state)
         VALUES ($1, $2, $3, $4, 'Cutover service', $5, $6,
                 '[]', '[]', false)",
    )
    .bind(release_agent_id)
    .bind(release_id)
    .bind(family_id)
    .bind(&source_agent_key)
    .bind(serde_json::json!({
        "executable": "bin/service",
        "arguments": [],
        "working_directory": "bin",
        "image_reference": ROOT_IMAGE,
        "requires_state": false,
        "policy_ceiling": {"vcpus": 1, "memory_mib": 512, "network": "disabled"}
    }))
    .bind([24_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("seed cutover release agent");

    let artifact = SERVICE_GATEWAY_HANDLER.as_bytes();
    tokio::fs::create_dir_all(artifact_root)
        .await
        .expect("cutover release artifact root");
    let artifact_path = artifact_root.join(storage_key.simple().to_string());
    tokio::fs::write(&artifact_path, artifact)
        .await
        .expect("cutover service artifact");
    let mut permissions = tokio::fs::metadata(&artifact_path)
        .await
        .expect("cutover service artifact metadata")
        .permissions();
    PermissionsExt::set_mode(&mut permissions, 0o555);
    tokio::fs::set_permissions(&artifact_path, permissions)
        .await
        .expect("cutover service artifact mode");
    let artifact_hash: [u8; 32] = Sha256::digest(artifact).into();
    sqlx::query(
        "INSERT INTO release_artifacts
           (id, release_id, path, kind, mode, content_hash, size_bytes,
            media_type, storage_key)
         VALUES ($1, $2, 'bin/service', 'executable', 365, $3, $4,
                 'application/octet-stream', $5)",
    )
    .bind(artifact_id)
    .bind(release_id)
    .bind(artifact_hash.as_slice())
    .bind(i64::try_from(artifact.len()).expect("cutover artifact length"))
    .bind(storage_key)
    .execute(pool)
    .await
    .expect("seed cutover service artifact");
    sqlx::query(
        "INSERT INTO gateway_revisions
            (id, gateway_id, project_id, repository_id, release_id,
             release_agent_id, release_agent_key, handler_contract, exposure,
             parameters, secret_slots, mailbox_slots, normalized_hash, created_by,
             service_loopback_port, service_readiness_path, service_health_path)
         VALUES ($1, $2, $3, $4, $5, $6, $7, 'http.service.v1', 'public',
                 '{}', '{}', '{}', $8, $9, 8080, $10, '/healthz')",
    )
    .bind(revision_id)
    .bind(fixture.gateway_id)
    .bind(project_id)
    .bind(repository_id)
    .bind(release_id)
    .bind(release_agent_id)
    .bind(&source_agent_key)
    .bind(normalized_hash.as_slice())
    .bind(publication_actor_id)
    .bind(readiness_path)
    .execute(pool)
    .await
    .expect("seed cutover service revision");
    for path in ["/service", "/service/identity", "/service/crash"] {
        sqlx::query(
            "INSERT INTO gateway_routes
               (id, gateway_revision_id, gateway_id, project_id, path, methods)
             VALUES ($1, $2, $3, $4, $5, ARRAY['GET'])",
        )
        .bind(uuid::Uuid::new_v4())
        .bind(revision_id)
        .bind(fixture.gateway_id)
        .bind(project_id)
        .bind(path)
        .execute(pool)
        .await
        .expect("seed cutover service route");
    }
    GatewayServiceGoldenFixture {
        gateway_id: fixture.gateway_id,
        revision_id,
    }
}

/// Waits for the daemon-owned service supervisor to prove readiness and make
/// the exact desired revision active. The query intentionally observes both
/// pointers and the fenced instance state, so a declaration-only Caddy route
/// cannot make the public request proof pass.
async fn wait_for_gateway_service_ready(
    pool: &sqlx::PgPool,
    fixture: &GatewayServiceGoldenFixture,
) -> uuid::Uuid {
    tokio::time::timeout(Duration::from_secs(120), async {
        loop {
            let row: Option<(uuid::Uuid, String, Option<uuid::Uuid>)> = sqlx::query_as(
                "SELECT instance.id, instance.state, gateway.active_revision_id
                   FROM gateway_service_instances AS instance
                   JOIN gateways AS gateway ON gateway.id = instance.gateway_id
                  WHERE instance.gateway_id = $1
                    AND instance.revision_id = $2
                  ORDER BY instance.created_at DESC
                  LIMIT 1",
            )
            .bind(fixture.gateway_id)
            .bind(fixture.revision_id)
            .fetch_optional(pool)
            .await
            .expect("read daemon-owned service readiness");
            if let Some((instance_id, state, active_revision_id)) = row {
                if state == "ready" && active_revision_id == Some(fixture.revision_id) {
                    return instance_id;
                }
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("daemon-owned service reaches Ready and active state")
}

/// Waits for the daemon to retain the crashed instance's redacted exit report,
/// finish its physical cleanup, and promote a replacement for the same
/// immutable revision.
async fn wait_for_gateway_service_crash_replacement(
    pool: &sqlx::PgPool,
    fixture: &GatewayServiceGoldenFixture,
    crashed_instance_id: uuid::Uuid,
) -> uuid::Uuid {
    tokio::time::timeout(Duration::from_secs(120), async {
        loop {
            let old: Option<GatewayServiceCrashEvidence> = sqlx::query_as(
                "SELECT state, failure_code, exit_code, exit_signal
                       FROM gateway_service_instances
                      WHERE id = $1 AND gateway_id = $2 AND revision_id = $3",
            )
            .bind(crashed_instance_id)
            .bind(fixture.gateway_id)
            .bind(fixture.revision_id)
            .fetch_optional(pool)
            .await
            .expect("read crashed persistent-service instance");
            let replacement: Option<uuid::Uuid> = sqlx::query_scalar(
                "SELECT instance.id
                   FROM gateway_service_instances AS instance
                   JOIN gateways AS gateway ON gateway.id = instance.gateway_id
                  WHERE instance.id <> $1
                    AND instance.gateway_id = $2
                    AND instance.revision_id = $3
                    AND instance.state = 'ready'
                    AND gateway.active_revision_id = $3
                  ORDER BY instance.created_at DESC
                  LIMIT 1",
            )
            .bind(crashed_instance_id)
            .bind(fixture.gateway_id)
            .bind(fixture.revision_id)
            .fetch_optional(pool)
            .await
            .expect("read replacement persistent-service instance");
            if let (
                Some((state, Some(failure_code), Some(exit_code), None)),
                Some(replacement_id),
            ) = (old, replacement)
            {
                if state == "cleaned" && failure_code == "unexpected_exit" && exit_code == 42 {
                    return replacement_id;
                }
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("daemon replaces crashed persistent service")
}

/// Sends the opt-in fixture crash request through public Caddy routing and
/// waits for the guest's acknowledged 503 before it exits with code 42.
async fn exercise_gateway_service_crash(public_url: &str) {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("bounded persistent-service crash client");
    let response = client
        .get(format!("{public_url}/gateway/service/crash"))
        .send()
        .await
        .expect("public persistent-service crash request");
    assert_eq!(response.status(), reqwest::StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        response
            .bytes()
            .await
            .expect("read persistent-service crash response"),
        "crashing"
    );
}

/// Sends two public requests through the real Caddy/daemon path and proves
/// they reached one long-lived guest process rather than two per-request VMs.
struct GatewayServiceRequestProof {
    pid: u64,
    startup_id: String,
}

async fn exercise_gateway_service_requests(public_url: &str) -> GatewayServiceRequestProof {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("bounded persistent-service client");
    let identity_url = format!("{public_url}/gateway/service/identity");
    let first = client
        .get(&identity_url)
        .send()
        .await
        .expect("first public persistent-service request")
        .error_for_status()
        .expect("first persistent-service request succeeds")
        .bytes()
        .await
        .expect("read first persistent-service identity");
    let second = client
        .get(&identity_url)
        .send()
        .await
        .expect("second public persistent-service request")
        .error_for_status()
        .expect("second persistent-service request succeeds")
        .bytes()
        .await
        .expect("read second persistent-service identity");
    let first: serde_json::Value =
        serde_json::from_slice(&first).expect("first service identity JSON");
    let second: serde_json::Value =
        serde_json::from_slice(&second).expect("second service identity JSON");
    let first_startup = first
        .get("startup_id")
        .and_then(serde_json::Value::as_str)
        .expect("first service response startup identity");
    let second_startup = second
        .get("startup_id")
        .and_then(serde_json::Value::as_str)
        .expect("second service response startup identity");
    assert_eq!(first_startup, second_startup);
    let first_pid = first
        .get("pid")
        .and_then(serde_json::Value::as_u64)
        .expect("first service response process identity");
    assert!(first_pid > 0, "first service response PID must be positive");
    let second_pid = second
        .get("pid")
        .and_then(serde_json::Value::as_u64)
        .expect("second service response process identity");
    assert_eq!(first_pid, second_pid);
    let first_count = first
        .get("request_count")
        .and_then(serde_json::Value::as_u64)
        .expect("first service response request count");
    let second_count = second
        .get("request_count")
        .and_then(serde_json::Value::as_u64)
        .expect("second service response request count");
    assert!(second_count > first_count);
    println!(
        "persistent-service-public identity_equal=true pid={first_pid} startup_id={first_startup} request_count={first_count}->{second_count}"
    );
    GatewayServiceRequestProof {
        pid: first_pid,
        startup_id: first_startup.to_owned(),
    }
}

/// Sends two identity requests to the published cooking service. Its small
/// sample binary reports only PID and startup identity, so this proof keeps
/// the request-count assertion private to the seeded integration fixture.
async fn exercise_published_cooking_service_identity(
    public_url: &str,
) -> GatewayServiceRequestProof {
    let client = published_cooking_service_client();
    let identity_url = format!("{public_url}/gateway/service/identity");
    let first = client
        .get(&identity_url)
        .send()
        .await
        .expect("first published cooking-service identity request")
        .error_for_status()
        .expect("first published cooking-service identity succeeds")
        .bytes()
        .await
        .expect("read first published cooking-service identity");
    let second = client
        .get(&identity_url)
        .send()
        .await
        .expect("second published cooking-service identity request")
        .error_for_status()
        .expect("second published cooking-service identity succeeds")
        .bytes()
        .await
        .expect("read second published cooking-service identity");
    let first: serde_json::Value =
        serde_json::from_slice(&first).expect("first published cooking-service identity JSON");
    let second: serde_json::Value =
        serde_json::from_slice(&second).expect("second published cooking-service identity JSON");
    let first_startup = first
        .get("startup_id")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty())
        .expect("first published cooking-service startup identity");
    let second_startup = second
        .get("startup_id")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty())
        .expect("second published cooking-service startup identity");
    assert_eq!(first_startup, second_startup);
    let first_pid = first
        .get("pid")
        .and_then(serde_json::Value::as_u64)
        .expect("first published cooking-service PID");
    assert!(
        first_pid > 0,
        "published cooking-service PID must be positive"
    );
    let second_pid = second
        .get("pid")
        .and_then(serde_json::Value::as_u64)
        .expect("second published cooking-service PID");
    assert_eq!(first_pid, second_pid);
    GatewayServiceRequestProof {
        pid: first_pid,
        startup_id: first_startup.to_owned(),
    }
}

fn published_cooking_service_client() -> reqwest::Client {
    let mut builder = reqwest::Client::builder().timeout(Duration::from_secs(10));
    if env::var("HEPHAESTUS_CADDY_TEST_TLS").as_deref() == Ok("1") {
        let ca_path =
            env::var("HEPHAESTUS_CADDY_TEST_CA_CERT").expect("joined Caddy TLS fixture CA path");
        let ca_pem = fs::read(&ca_path).expect("read joined Caddy TLS fixture CA");
        let certificate =
            reqwest::Certificate::from_pem(&ca_pem).expect("parse joined Caddy TLS fixture CA");
        builder = builder
            .tls_built_in_root_certs(false)
            .add_root_certificate(certificate);
    }
    builder
        .build()
        .expect("bounded published cooking-service client")
}

async fn exercise_published_cooking_service_metadata(public_url: &str) {
    let client = published_cooking_service_client();
    let metadata_url = format!("{public_url}/gateway/service/metadata");
    for (label, request) in [
        ("normal", client.get(&metadata_url)),
        (
            "forged forwarding",
            client
                .get(&metadata_url)
                .header(reqwest::header::HOST, "attacker.golden.invalid")
                .header("Forwarded", "for=198.51.100.7;host=attacker.golden.invalid")
                .header("X-Forwarded-For", "198.51.100.7")
                .header("X-Forwarded-Host", "attacker.golden.invalid")
                .header("X-Forwarded-Proto", "http"),
        ),
    ] {
        let response = request
            .send()
            .await
            .unwrap_or_else(|error| panic!("{label} published metadata request: {error}"))
            .error_for_status()
            .unwrap_or_else(|error| panic!("{label} published metadata request succeeds: {error}"))
            .bytes()
            .await
            .unwrap_or_else(|error| panic!("read {label} published metadata response: {error}"));
        let metadata: serde_json::Value = serde_json::from_slice(&response)
            .unwrap_or_else(|error| panic!("{label} published metadata JSON: {error}"));
        let object = metadata
            .as_object()
            .unwrap_or_else(|| panic!("{label} published metadata object"));
        let expected_keys = [
            "host_matches_expected",
            "forwarded_present",
            "x_forwarded_for_present",
            "x_forwarded_host_present",
            "x_forwarded_proto_present",
        ]
        .into_iter()
        .map(String::from)
        .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            object
                .keys()
                .cloned()
                .collect::<std::collections::BTreeSet<_>>(),
            expected_keys,
            "{label} metadata schema"
        );
        assert_eq!(
            object
                .get("host_matches_expected")
                .and_then(serde_json::Value::as_bool),
            Some(true),
            "{label} request keeps the configured authority"
        );
        for key in [
            "forwarded_present",
            "x_forwarded_for_present",
            "x_forwarded_host_present",
            "x_forwarded_proto_present",
        ] {
            assert_eq!(
                object.get(key).and_then(serde_json::Value::as_bool),
                Some(false),
                "{label} request must not expose caller forwarding header {key}"
            );
        }
    }
}

/// Runs the published service's bounded guest isolation diagnostic through the
/// real Caddy public listener, then proves the public listener cannot serve the
/// separate Caddy administration API, including with a forged admin Host.
async fn exercise_published_cooking_service_isolation(public_url: &str, admin_url: &str) {
    let public_port = published_public_loopback_port(public_url, "public Caddy URL");
    let admin_port = loopback_port(admin_url, "admin Caddy URL");
    assert_ne!(public_port, admin_port);
    assert_ne!(public_port, 8080);
    assert_ne!(admin_port, 8080);

    let client = published_cooking_service_client();
    let diagnostic = client
        .get(format!(
            "{public_url}/gateway/service/isolation?admin_port={admin_port}&public_port={public_port}"
        ))
        .send()
        .await
        .expect("published cooking-service isolation request")
        .error_for_status()
        .expect("published cooking-service isolation request succeeds")
        .bytes()
        .await
        .expect("read published cooking-service isolation response");
    let diagnostic: serde_json::Value =
        serde_json::from_slice(&diagnostic).expect("published cooking-service isolation JSON");
    let object = diagnostic
        .as_object()
        .expect("published isolation response object");
    let expected_keys = [
        "schema_version",
        "own_loopback_ok",
        "admin_loopback_blocked",
        "public_loopback_blocked",
        "metadata_blocked",
        "test_net_blocked",
        "runtime_authority_env_absent",
        "runtime_authority_path_absent",
        "broker_socket_absent",
        "secret_mount_absent",
        "control_surface_ok",
    ]
    .into_iter()
    .map(String::from)
    .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        object
            .keys()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>(),
        expected_keys,
        "published isolation response must have exactly the reviewed schema"
    );
    assert_eq!(
        object
            .get("schema_version")
            .and_then(serde_json::Value::as_u64),
        Some(1)
    );
    for key in [
        "own_loopback_ok",
        "admin_loopback_blocked",
        "public_loopback_blocked",
        "metadata_blocked",
        "test_net_blocked",
        "runtime_authority_env_absent",
        "runtime_authority_path_absent",
        "broker_socket_absent",
        "secret_mount_absent",
        "control_surface_ok",
    ] {
        assert_eq!(
            object.get(key).and_then(serde_json::Value::as_bool),
            Some(true),
            "published isolation field {key}"
        );
    }

    for host in [None, Some(format!("127.0.0.1:{admin_port}"))] {
        let mut request = client.get(format!("{public_url}/config/"));
        if let Some(host) = host {
            request = request.header(reqwest::header::HOST, host);
        }
        let response = request
            .send()
            .await
            .expect("public Caddy internal-config probe");
        assert_eq!(
            response.status(),
            reqwest::StatusCode::NOT_FOUND,
            "public Caddy listener must deny /config/"
        );
        response
            .bytes()
            .await
            .expect("read public Caddy internal-config denial");
    }
}

fn published_public_loopback_port(url: &str, label: &str) -> u16 {
    let url =
        reqwest::Url::parse(url).unwrap_or_else(|error| panic!("{label} is invalid: {error}"));
    let expected_scheme = if env::var("HEPHAESTUS_CADDY_TEST_TLS").as_deref() == Ok("1") {
        "https"
    } else {
        "http"
    };
    assert_eq!(
        url.scheme(),
        expected_scheme,
        "{label} must use the configured disposable harness scheme"
    );
    loopback_port_from_url(&url, label)
}

fn loopback_port(url: &str, label: &str) -> u16 {
    let url =
        reqwest::Url::parse(url).unwrap_or_else(|error| panic!("{label} is invalid: {error}"));
    assert_eq!(url.scheme(), "http", "{label} must use HTTP administration");
    loopback_port_from_url(&url, label)
}

fn loopback_port_from_url(url: &reqwest::Url, label: &str) -> u16 {
    assert_eq!(
        url.host_str(),
        Some("127.0.0.1"),
        "{label} must use the joined loopback listener"
    );
    url.port()
        .unwrap_or_else(|| panic!("{label} has no explicit port"))
}

#[cfg(feature = "test-fixtures")]
async fn assert_guest_gateway_service_log_retained(
    pool: &sqlx::PgPool,
    proof: &GatewayServiceGuestLogProof,
) {
    let epoch: (i64, i64, i64) = sqlx::query_as(
        "SELECT fencing_token, acknowledged_through, retained_chunks
           FROM gateway_service_log_epochs
          WHERE instance_id = $1 AND gateway_id = $2 AND revision_id = $3
            AND project_id = $4 AND fencing_token = $5",
    )
    .bind(proof.instance_id)
    .bind(proof.gateway_id)
    .bind(proof.revision_id)
    .bind(proof.project_id)
    .bind(proof.fencing_token)
    .fetch_one(pool)
    .await
    .expect("read retained guest service-log epoch");
    assert_eq!(epoch.0, proof.fencing_token);
    assert!(epoch.1 >= 0);

    let rows: Vec<(i64, String, Vec<u8>)> = sqlx::query_as(
        "SELECT sequence, stream, bytes
           FROM gateway_service_log_chunks
          WHERE instance_id = $1 AND gateway_id = $2 AND revision_id = $3
            AND project_id = $4 AND fencing_token = $5
          ORDER BY sequence",
    )
    .bind(proof.instance_id)
    .bind(proof.gateway_id)
    .bind(proof.revision_id)
    .bind(proof.project_id)
    .bind(proof.fencing_token)
    .fetch_all(pool)
    .await
    .expect("read retained guest service-log chunks");
    assert!(
        !rows.is_empty(),
        "guest service-log chunks were not retained"
    );
    assert_eq!(
        usize::try_from(epoch.2).expect("retained chunk count fits usize"),
        rows.len()
    );

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut previous_sequence = None;
    for (sequence, stream, bytes) in rows {
        if let Some(previous_sequence) = previous_sequence {
            assert!(sequence > previous_sequence);
        }
        previous_sequence = Some(sequence);
        match stream.as_str() {
            "stdout" => stdout.extend_from_slice(&bytes),
            "stderr" => stderr.extend_from_slice(&bytes),
            other => panic!("unexpected retained guest service-log stream: {other}"),
        }
    }
    assert!(
        stdout.starts_with(&proof.stdout),
        "pre-shutdown stdout must remain retained in stream order"
    );
    assert!(
        stderr.starts_with(&proof.stderr),
        "pre-shutdown stderr must remain retained in stream order"
    );
    assert_eq!(
        marker_count(&stdout, GUEST_SERVICE_LOG_STDOUT_MARKER),
        1,
        "retained stdout marker must remain unique"
    );
    assert_eq!(
        marker_count(&stderr, GUEST_SERVICE_LOG_STDERR_MARKER),
        1,
        "retained stderr marker must remain unique"
    );
}

#[cfg(feature = "test-fixtures")]
// Keep the complete SQL authority fixture together so its persisted scopes and
// application-role assertions remain reviewable as one setup boundary.
#[allow(
    clippy::cognitive_complexity,
    clippy::too_many_arguments,
    clippy::too_many_lines
)]
async fn prepare_gateway_service_log_rpc(
    pool: &sqlx::PgPool,
    running: &hephaestus_app::RunningHephaestus,
    gateway_id: uuid::Uuid,
    revision_id: uuid::Uuid,
    instance_id: uuid::Uuid,
    project_id: uuid::Uuid,
) -> (
    control_plane_postgres::ControlPlanePool,
    GatewayServiceLogRpcFixture,
) {
    let composed_pool = running.application_pool_for_test();
    let current_user: String = sqlx::query_scalar("SELECT current_user")
        .fetch_one(&composed_pool)
        .await
        .expect("read composed service log RPC pool role");
    assert_eq!(current_user, "hephaestus_app");
    let error = sqlx::query("SELECT retained_bytes FROM gateway_service_log_project_usage")
        .fetch_optional(&composed_pool)
        .await
        .expect_err("application role cannot read worker-only project usage");
    let code = error
        .as_database_error()
        .and_then(sqlx::error::DatabaseError::code);
    assert_eq!(code.as_deref(), Some("42501"));
    let mut fixture_transaction = pool.begin().await.expect("begin service log RPC fixture");
    let (fencing_token, first_sequence): (i64, i64) = sqlx::query_as(
        "SELECT instance.fencing_token,
                COALESCE(MAX(chunks.sequence), -1) + 1
           FROM gateway_service_instances AS instance
           LEFT JOIN gateway_service_log_chunks AS chunks
             ON chunks.instance_id = instance.id
            AND chunks.gateway_id = instance.gateway_id
            AND chunks.revision_id = instance.revision_id
            AND chunks.fencing_token = instance.fencing_token
          WHERE instance.id = $1
            AND instance.gateway_id = $2
            AND instance.revision_id = $3
          GROUP BY instance.fencing_token",
    )
    .bind(instance_id)
    .bind(gateway_id)
    .bind(revision_id)
    .fetch_one(&mut *fixture_transaction)
    .await
    .expect("read ready service log RPC scope");
    assert!(fencing_token > 0);
    let baseline_epoch: Option<(i64, i64, i64)> = sqlx::query_as(
        "SELECT acknowledged_through, retained_bytes, retained_chunks
           FROM gateway_service_log_epochs
          WHERE instance_id = $1 AND gateway_id = $2 AND revision_id = $3
            AND project_id = $4 AND fencing_token = $5",
    )
    .bind(instance_id)
    .bind(gateway_id)
    .bind(revision_id)
    .bind(project_id)
    .bind(fencing_token)
    .fetch_optional(&mut *fixture_transaction)
    .await
    .expect("read baseline service log RPC metadata");
    let (baseline_acknowledged, baseline_bytes, baseline_chunks) =
        baseline_epoch.unwrap_or((-1, 0, 0));

    sqlx::query(
        "INSERT INTO gateway_service_log_project_usage (project_id)
         VALUES ($1)
         ON CONFLICT (project_id) DO NOTHING",
    )
    .bind(project_id)
    .execute(&mut *fixture_transaction)
    .await
    .expect("seed service log RPC usage");
    let inserted_epoch: Option<i64> = sqlx::query_scalar(
        "INSERT INTO gateway_service_log_epochs
            (instance_id, gateway_id, revision_id, project_id, fencing_token)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (instance_id, fencing_token) DO NOTHING
         RETURNING fencing_token",
    )
    .bind(instance_id)
    .bind(gateway_id)
    .bind(revision_id)
    .bind(project_id)
    .bind(fencing_token)
    .fetch_optional(&mut *fixture_transaction)
    .await
    .expect("seed service log RPC epoch");
    if inserted_epoch.is_some() {
        sqlx::query(
            "UPDATE gateway_service_log_project_usage
                SET retained_epochs = retained_epochs + 1, updated_at = now()
              WHERE project_id = $1",
        )
        .bind(project_id)
        .execute(&mut *fixture_transaction)
        .await
        .expect("update service log RPC epoch usage");
    }
    let payloads = [
        b"rpc-service-log-first".as_slice(),
        b"rpc-service-log-second".as_slice(),
    ];
    let payload_bytes = payloads
        .iter()
        .map(|payload| i64::try_from(payload.len()).expect("small log payload"))
        .sum::<i64>();
    for (offset, payload) in payloads.iter().enumerate() {
        let sequence = first_sequence + i64::try_from(offset).expect("small log sequence");
        sqlx::query(
            "INSERT INTO gateway_service_log_chunks
                (instance_id, gateway_id, revision_id, project_id, fencing_token,
                 sequence, stream, observed_at, bytes)
             VALUES ($1, $2, $3, $4, $5, $6, 'stdout', now(), $7)",
        )
        .bind(instance_id)
        .bind(gateway_id)
        .bind(revision_id)
        .bind(project_id)
        .bind(fencing_token)
        .bind(sequence)
        .bind(*payload)
        .execute(&mut *fixture_transaction)
        .await
        .expect("seed service log RPC payload");
        sqlx::query(
            "UPDATE gateway_service_log_epochs
                SET acknowledged_through = GREATEST(acknowledged_through, $6),
                    retained_bytes = retained_bytes + $7,
                    retained_chunks = retained_chunks + 1,
                    updated_at = now()
              WHERE instance_id = $1 AND gateway_id = $2 AND revision_id = $3
                AND project_id = $4 AND fencing_token = $5",
        )
        .bind(instance_id)
        .bind(gateway_id)
        .bind(revision_id)
        .bind(project_id)
        .bind(fencing_token)
        .bind(sequence)
        .bind(i64::try_from(payload.len()).expect("small log payload"))
        .execute(&mut *fixture_transaction)
        .await
        .expect("update service log RPC metadata");
        sqlx::query(
            "UPDATE gateway_service_log_project_usage
                SET retained_bytes = retained_bytes + $2,
                    retained_chunks = retained_chunks + 1,
                    storage_dropped_chunks = 7,
                    storage_dropped_bytes = 123,
                    updated_at = now()
              WHERE project_id = $1",
        )
        .bind(project_id)
        .bind(i64::try_from(payload.len()).expect("small log payload"))
        .execute(&mut *fixture_transaction)
        .await
        .expect("update service log RPC project usage");
    }
    fixture_transaction
        .commit()
        .await
        .expect("commit service log RPC fixture");

    let foreign_owner = uuid::Uuid::new_v4();
    let foreign_organization = uuid::Uuid::new_v4();
    let foreign_project = uuid::Uuid::new_v4();
    sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, 'RPC Log Foreign Owner')")
        .bind(foreign_owner)
        .execute(pool)
        .await
        .expect("seed foreign service-log owner");
    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, $2)")
        .bind(foreign_organization)
        .bind(format!("rpc-log-foreign-{foreign_organization}"))
        .execute(pool)
        .await
        .expect("seed foreign service-log organization");
    sqlx::query(
        "INSERT INTO organization_members (organization_id, user_id, role)
         VALUES ($1, $2, 'owner')",
    )
    .bind(foreign_organization)
    .bind(foreign_owner)
    .execute(pool)
    .await
    .expect("seed foreign service-log organization owner");
    sqlx::query("INSERT INTO projects (id, organization_id, name) VALUES ($1, $2, $3)")
        .bind(foreign_project)
        .bind(foreign_organization)
        .bind(format!("rpc-log-foreign-project-{foreign_project}"))
        .execute(pool)
        .await
        .expect("seed foreign service-log project");
    sqlx::query(
        "INSERT INTO gateway_service_log_project_usage
            (project_id, storage_dropped_chunks, storage_dropped_bytes)
         VALUES ($1, 99, 999)",
    )
    .bind(foreign_project)
    .execute(pool)
    .await
    .expect("seed foreign service-log metadata");
    let member_id = uuid::Uuid::new_v4();
    sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, 'RPC Log Member')")
        .bind(member_id)
        .execute(pool)
        .await
        .expect("seed service log RPC member");
    let member_browser_session = seed_golden_browser_session(
        pool,
        UserId::from_uuid(member_id),
        &golden_issuer(),
        &format!("member-{member_id}"),
    )
    .await;
    sqlx::query("INSERT INTO project_maintainers (project_id, user_id) VALUES ($1, $2)")
        .bind(project_id)
        .bind(member_id)
        .execute(pool)
        .await
        .expect("grant service log RPC member");
    (
        composed_pool,
        GatewayServiceLogRpcFixture {
            gateway_id,
            revision_id,
            instance_id,
            project_id,
            fencing_token,
            first_sequence,
            baseline_acknowledged,
            baseline_bytes,
            baseline_chunks,
            payloads: [payloads[0].to_vec(), payloads[1].to_vec()],
            payload_bytes,
            foreign_project,
            member_id,
            member_browser_session,
        },
    )
}

/// Seeds immutable gateway route and host-only inbound secret authority.
// The fixture deliberately names each persisted authority boundary explicitly.
// Keeping this one setup transaction-shaped makes the golden authority graph
// reviewable without hiding a bound mailbox or grant in a generic helper.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
async fn seed_gateway_brokered_route(
    pool: &sqlx::PgPool,
    actor: UserId,
    project_id: uuid::Uuid,
    repository_id: uuid::Uuid,
    release_id: uuid::Uuid,
    release_agent_id: uuid::Uuid,
    import_id: uuid::Uuid,
    version_id: uuid::Uuid,
    mailbox_id: MailboxId,
) -> GatewayGoldenFixture {
    let gateway_id = uuid::Uuid::new_v4();
    let revision_id = uuid::Uuid::new_v4();
    let route_id = uuid::Uuid::new_v4();
    let binding_id = uuid::Uuid::new_v4();
    sqlx::query(
        "INSERT INTO gateways
           (id, project_id, repository_id, name, lifecycle, created_by)
         VALUES ($1, $2, $3, 'golden-gateway', 'enabled', $4)",
    )
    .bind(gateway_id)
    .bind(project_id)
    .bind(repository_id)
    .bind(actor.as_uuid())
    .execute(pool)
    .await
    .expect("seed gateway");
    sqlx::query(
        "INSERT INTO gateway_revisions
           (id, gateway_id, project_id, repository_id, release_id, release_agent_id,
            release_agent_key, handler_contract, exposure, parameters, secret_slots, mailbox_slots,
            normalized_hash, created_by)
         VALUES ($1, $2, $3, $4, $5, $6, 'golden-gateway', 'http.v1', 'public',
                 $9, ARRAY['webhook'], $10, $7, $8)",
    )
    .bind(revision_id)
    .bind(gateway_id)
    .bind(project_id)
    .bind(repository_id)
    .bind(release_id)
    .bind(release_agent_id)
    .bind([8_u8; 32].as_slice())
    .bind(actor.as_uuid())
    .bind(if cooking::enabled() {
        serde_json::json!({"inbound_placeholder": cooking_builds::cooking_inbound_placeholder(version_id), "alice_provider_id":1001, "bob_provider_id":1002})
    } else {
        serde_json::json!({})
    })
    .bind(vec![if cooking::enabled() {
        "cooking_requests"
    } else {
        "deliver"
    }])
    .execute(pool)
    .await
    .expect("seed immutable gateway revision");
    sqlx::query(
        "INSERT INTO gateway_routes
           (id, gateway_revision_id, gateway_id, project_id, path, methods)
         VALUES ($1, $2, $3, $4, $5, ARRAY['POST'])",
    )
    .bind(route_id)
    .bind(revision_id)
    .bind(gateway_id)
    .bind(project_id)
    .bind(if cooking::enabled() {
        "/cooking/telegram"
    } else {
        "/brokered"
    })
    .execute(pool)
    .await
    .expect("seed gateway route");
    sqlx::query("UPDATE gateways SET active_revision_id = $2 WHERE id = $1")
        .bind(gateway_id)
        .bind(revision_id)
        .execute(pool)
        .await
        .expect("activate gateway revision");
    sqlx::query(
        "INSERT INTO gateway_secret_bindings
           (id, gateway_id, gateway_revision_id, import_id, slot_key,
            secret_version_id, status, normalized_hash)
         VALUES ($1, $2, $3, $4, 'webhook', $5, 'active', $6)",
    )
    .bind(binding_id)
    .bind(gateway_id)
    .bind(revision_id)
    .bind(import_id)
    .bind(version_id)
    .bind([9_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("seed gateway secret binding");
    let mailbox_binding_id = uuid::Uuid::new_v4();
    sqlx::query(
        "INSERT INTO gateway_mailbox_bindings
           (id, gateway_revision_id, gateway_id, project_id, slot_key, mailbox_id,
            producer_id, created_by)
         VALUES ($1, $2, $3, $4, $7, $5, 'golden-gateway', $6)",
    )
    .bind(mailbox_binding_id)
    .bind(revision_id)
    .bind(gateway_id)
    .bind(project_id)
    .bind(mailbox_id.as_uuid())
    .bind(actor.as_uuid())
    .bind(if cooking::enabled() {
        "cooking_requests"
    } else {
        "deliver"
    })
    .execute(pool)
    .await
    .expect("bind gateway fixture mailbox");
    let grant_id = uuid::Uuid::new_v4();
    sqlx::query(
        "INSERT INTO gateway_mailbox_binding_grants (id, binding_id, status, granted_by)
         VALUES ($1, $2, 'active', $3)",
    )
    .bind(grant_id)
    .bind(mailbox_binding_id)
    .bind(actor.as_uuid())
    .execute(pool)
    .await
    .expect("grant gateway fixture mailbox publication");
    sqlx::query(
        "INSERT INTO gateway_brokered_secret_rules
           (id, binding_id, gateway_revision_id, gateway_route_id, header_name, normalized_hash)
         VALUES ($1, $2, $3, $4, $6, $5)",
    )
    .bind(uuid::Uuid::new_v4())
    .bind(binding_id)
    .bind(revision_id)
    .bind(route_id)
    .bind([10_u8; 32].as_slice())
    .bind(if cooking::enabled() {
        "x-telegram-bot-api-secret-token"
    } else {
        "x-webhook-secret"
    })
    .execute(pool)
    .await
    .expect("seed gateway brokered inbound rule");
    GatewayGoldenFixture {
        mailbox_id,
        grant_id,
    }
}

/// Creates the complete durable secret authority that the daemon resolves at
/// mailbox dispatch. The only plaintext in this test is passed directly into
/// the encrypted secret-store boundary and is then asserted only at the local
/// TLS upstream.
#[allow(clippy::too_many_lines)]
async fn seed_brokered_https_fixture(
    pool: &sqlx::PgPool,
    actor: UserId,
    organization_id: OrganizationId,
    project_id: uuid::Uuid,
    instance: &SeededInstance,
) -> BrokeredFixture {
    sqlx::query(
        "INSERT INTO project_secret_roles (project_id, user_id, role)
          VALUES ($1, $2, 'secret_manager')",
    )
    .bind(project_id)
    .bind(actor.as_uuid())
    .execute(pool)
    .await
    .expect("authorize golden owner to manage project secret imports");

    let identity = AuthenticatedIdentity::new(
        actor,
        golden_issuer(),
        String::from("golden-subject"),
        serde_json::json!({}),
        RequestId::new(),
    );
    let service = SecretService::new(
        pool.clone(),
        EncryptedStore::new(
            LocalKeyProvider::new("golden/v1", [("golden/v1", [17_u8; 32])])
                .expect("golden secret key"),
        ),
        Arc::new(PostgresMelangeAuthorizer),
    );
    let secret_id = SecretId::new();
    let version_id = SecretVersionId::new();
    service
        .create(
            &identity,
            CreateSecret {
                command_key: secret_command_key("create", secret_id.as_uuid()),
                secret_id,
                version_id,
                owner: SecretOwner::Organization(organization_id),
                name: SecretName::parse(format!("golden_broker_{secret_id}"))
                    .expect("fixture secret name"),
                allowed_delivery_modes: vec![DeliveryMode::Brokered],
                value: SecretValue::new(BROKERED_E2E_SENTINEL).expect("fixture secret value"),
            },
        )
        .await
        .expect("create encrypted brokered fixture secret");
    let grant_id = SecretGrantId::new();
    let import_id = SecretImportId::new();
    service
        .grant_and_accept_import(
            &identity,
            GrantAndAcceptSecretImport {
                command_key: secret_command_key("grant-accept", import_id.as_uuid()),
                grant_id,
                secret_id,
                target: SecretTarget::Project(ProjectId::from_uuid(project_id)),
                policy: SecretUsePolicy {
                    delivery_modes: vec![DeliveryMode::Brokered],
                    phases: vec![ExecutionPhase::Normal],
                    destinations: vec![String::from("api.example.test")],
                },
                expires_at: None,
                import_id,
                alias: SecretAlias::parse("golden_broker").expect("fixture import alias"),
            },
        )
        .await
        .expect("grant and import brokered fixture secret");
    let binding_id = AgentSecretBindingId::new();
    let bound_revision = release_domain::AgentInstanceRevisionId::new();
    service
        .bind_secret(
            &identity,
            BindSecret {
                command_key: secret_command_key("bind", binding_id.as_uuid()),
                binding_id,
                instance_id: release_domain::AgentInstanceId::from_uuid(instance.instance),
                expected_revision_id: release_domain::AgentInstanceRevisionId::from_uuid(
                    instance.revision,
                ),
                new_revision_id: bound_revision,
                import_id,
                slot: SecretSlotKey::parse("model").expect("fixture slot"),
                mode: DeliveryMode::Brokered,
                phases: vec![ExecutionPhase::Normal],
                attachment_ids: vec![instance.attachment],
                destinations: vec![String::from("api.example.test")],
            },
        )
        .await
        .expect("bind brokered fixture secret to immutable release revision");
    service
        .declare_brokered_https_rule(
            &identity,
            DeclareBrokeredHttpsRule {
                command_key: secret_command_key("declare-rule", BROKERED_E2E_RULE_ID),
                rule_id: BROKERED_E2E_RULE_ID,
                binding_id,
                destination: String::from("https://api.example.test"),
                header: String::from("authorization"),
                header_prefix: Some(String::from("Bearer ")),
            },
        )
        .await
        .expect("declare immutable brokered HTTPS fixture rule");
    let rule = BrokeredSecretRule {
        id: BrokeredSecretRuleId::from_uuid(BROKERED_E2E_RULE_ID),
        binding_id: binding_id.as_uuid(),
        instance_revision_id: bound_revision.as_uuid(),
        secret_version_id: version_id.as_uuid(),
        destination: Some(
            ExactHttpsOrigin::parse("https://api.example.test").expect("fixture destination"),
        ),
        location: HttpInjectionLocation::OutboundHeaderPrefix {
            header: HeaderName::parse("authorization").expect("fixture header"),
            prefix: String::from("Bearer "),
        },
        gateway_route_id: None,
    };
    BrokeredFixture {
        upstream: BrokeredTlsUpstream::start(rule).await,
        import_id: import_id.as_uuid(),
        version_id: version_id.as_uuid(),
    }
}

struct BrokeredFixture {
    upstream: BrokeredTlsUpstream,
    import_id: uuid::Uuid,
    version_id: uuid::Uuid,
}

fn secret_command_key(operation: &str, id: uuid::Uuid) -> SecretCommandKey {
    SecretCommandKey::derive(operation, &[id.as_bytes()])
}

/// Certificate-valid loopback upstream available only to this daemon golden.
/// Its local address is accepted solely through `secret-broker`'s feature-gated
/// test fixture; the production registry still rejects private pins.
struct BrokeredTlsUpstream {
    adapter: Arc<dyn secret_application::BrokerAdapter>,
    cooking_registry: Option<Arc<cooking::CookingAdapters>>,
    rule_adapters:
        Arc<std::collections::HashMap<uuid::Uuid, Arc<dyn secret_application::BrokerAdapter>>>,
    observed: Arc<AtomicBool>,
    server: tokio::task::JoinHandle<()>,
    update_barrier: Option<Arc<CookingUpdateBarrier>>,
    revocation_barrier: Option<Arc<CookingUpdateBarrier>>,
}

/// Controlled model response barrier used by the cooking update scenario.
/// The model request enters before the v1 drain and resumes only after the
/// test has observed the durable drain state.
struct CookingUpdateBarrier {
    entered: AtomicBool,
    entered_notify: Notify,
    release: Notify,
}

impl CookingUpdateBarrier {
    fn new() -> Self {
        Self {
            entered: AtomicBool::new(false),
            entered_notify: Notify::new(),
            release: Notify::new(),
        }
    }

    async fn wait_entered(&self) {
        while !self.entered.load(Ordering::Acquire) {
            self.entered_notify.notified().await;
        }
    }

    fn mark_entered(&self) {
        self.entered.store(true, Ordering::Release);
        self.entered_notify.notify_one();
    }

    fn release(&self) {
        self.release.notify_one();
    }
}

#[cfg(test)]
#[tokio::test]
async fn cooking_update_barrier_waits_for_release() {
    let barrier = Arc::new(CookingUpdateBarrier::new());
    let upstream = Arc::new(BrokeredTlsUpstream {
        adapter: Arc::new(DenyingBrokerAdapter),
        rule_adapters: Arc::new(std::collections::HashMap::new()),
        cooking_registry: None,
        observed: Arc::new(AtomicBool::new(false)),
        server: tokio::spawn(async {}),
        update_barrier: Some(Arc::clone(&barrier)),
        revocation_barrier: None,
    });
    let waiter = Arc::clone(&upstream);
    let task = tokio::spawn(async move {
        waiter.wait_update_v1_entered().await;
    });
    barrier.mark_entered();
    tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .expect("barrier waiter timeout")
        .expect("barrier waiter task");
    upstream.release_update_v1();
}

#[cfg(test)]
#[tokio::test]
async fn cooking_update_barrier_buffers_release_before_waiter_registration() {
    let barrier = CookingUpdateBarrier::new();
    barrier.mark_entered();
    barrier.wait_entered().await;
    barrier.release();
    tokio::time::timeout(Duration::from_secs(1), barrier.release.notified())
        .await
        .expect("release notification should retain its permit");
}

impl BrokeredTlsUpstream {
    async fn wait_update_v1_entered(&self) {
        if let Some(barrier) = &self.update_barrier {
            barrier.wait_entered().await;
        } else {
            panic!("cooking update barrier is not enabled");
        }
    }

    async fn wait_revocation_entered(&self) {
        if let Some(barrier) = &self.revocation_barrier {
            barrier.wait_entered().await;
        } else {
            panic!("cooking revocation barrier is not enabled");
        }
    }

    fn release_revocation(&self) {
        if let Some(barrier) = &self.revocation_barrier {
            barrier.release();
        } else {
            panic!("cooking revocation barrier is not enabled");
        }
    }

    fn release_update_v1(&self) {
        if let Some(barrier) = &self.update_barrier {
            barrier.release();
        } else {
            panic!("cooking update barrier is not enabled");
        }
    }

    async fn start(rule: BrokeredSecretRule) -> Self {
        let _already_installed = rustls::crypto::ring::default_provider().install_default();
        let mut ca_parameters = rcgen::CertificateParams::default();
        ca_parameters.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        let ca_key = rcgen::KeyPair::generate().expect("generate golden upstream CA key");
        let ca = ca_parameters
            .self_signed(&ca_key)
            .expect("self-sign golden upstream CA");
        let leaf_parameters = rcgen::CertificateParams::new(vec![String::from("api.example.test")])
            .expect("golden upstream DNS identity");
        let leaf_key = rcgen::KeyPair::generate().expect("generate golden upstream leaf key");
        let leaf = leaf_parameters
            .signed_by(&leaf_key, &ca, &ca_key)
            .expect("sign golden upstream leaf");
        let tls = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(
                vec![rustls::pki_types::CertificateDer::from(leaf.der().to_vec())],
                rustls::pki_types::PrivateKeyDer::Pkcs8(leaf_key.serialize_der().into()),
            )
            .expect("golden upstream TLS configuration");
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind golden TLS upstream");
        let port = listener
            .local_addr()
            .expect("golden TLS upstream address")
            .port();
        let observed = Arc::new(AtomicBool::new(false));
        let observed_server = Arc::clone(&observed);
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accept golden TLS request");
            let mut stream = TlsAcceptor::from(Arc::new(tls))
                .accept(stream)
                .await
                .expect("verify golden TLS handshake");
            let mut request = Vec::with_capacity(512);
            loop {
                let mut chunk = [0_u8; 512];
                let read = stream.read(&mut chunk).await.expect("read golden request");
                assert_ne!(read, 0, "golden TLS request ended before headers");
                request.extend_from_slice(&chunk[..read]);
                assert!(
                    request.len() <= 8 * 1024,
                    "golden TLS request exceeded bound"
                );
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            let request = std::str::from_utf8(&request).expect("golden HTTPS request is UTF-8");
            assert!(request.starts_with("GET /v1/probe HTTP/1.1\r\n"));
            assert!(request.contains(&format!(
                "authorization: Bearer {BROKERED_E2E_SENTINEL}\r\n"
            )));
            assert!(!request.contains("heph-placeholder:"));
            observed_server.store(true, Ordering::SeqCst);
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 15\r\nConnection: close\r\n\r\nbrokered-e2e-ok")
                .await
                .expect("respond to golden brokered request");
        });
        let adapter = BrokeredHttpsAdapterRegistry::test_only_local_trusted(
            rule,
            port,
            "127.0.0.1".parse().expect("golden loopback pin"),
            ca.pem().as_bytes(),
        )
        .expect("configure locally trusted pinned broker registry");
        let adapter: Arc<dyn secret_application::BrokerAdapter> = Arc::new(adapter);
        Self {
            adapter: Arc::clone(&adapter),
            cooking_registry: None,
            rule_adapters: Arc::new(std::collections::HashMap::from([(
                BROKERED_E2E_RULE_ID,
                adapter,
            )])),
            observed,
            server,
            update_barrier: None,
            revocation_barrier: None,
        }
    }

    fn adapter(&self) -> Arc<dyn secret_application::BrokerAdapter> {
        Arc::clone(&self.adapter)
    }

    pub(crate) fn register_rule_copies(&self, copies: &[(uuid::Uuid, uuid::Uuid)]) {
        self.cooking_registry
            .as_ref()
            .expect("cooking TLS registry")
            .register_rule_copies(copies);
    }

    /// Combines rule-keyed adapters before a restart so canonical and crash
    /// instances share one broker boundary without destination guessing.
    pub(crate) fn combined_adapter(
        &self,
        other: &Self,
    ) -> Arc<dyn secret_application::BrokerAdapter> {
        let mut adapters = self.cooking_registry.as_ref().map_or_else(
            || (*self.rule_adapters).clone(),
            |registry| registry.snapshot(),
        );
        adapters.extend(
            other
                .cooking_registry
                .as_ref()
                .map_or_else(
                    || (*other.rule_adapters).clone(),
                    |registry| registry.snapshot(),
                )
                .iter()
                .map(|(id, adapter)| (*id, Arc::clone(adapter))),
        );
        Arc::new(cooking::CookingAdapters::new(adapters))
    }

    pub(crate) async fn wait_complete(self) -> Result<(), tokio::task::JoinError> {
        self.server.await
    }

    async fn assert_substituted_request(self) {
        tokio::time::timeout(Duration::from_secs(10), self.server)
            .await
            .expect("golden TLS upstream request timeout")
            .expect("golden TLS upstream task");
        assert!(
            self.observed.load(Ordering::SeqCst),
            "upstream did not receive the substituted HTTPS request"
        );
    }
}

/// Emits bounded, payload-free state when the production golden run times out.
/// This is intentionally test-only: it helps distinguish an un-dispatched
/// command from a persisted guest failure without exposing guest logs or
/// provider error strings (which could contain paths or secret material).
// The diagnostic deliberately keeps each bounded status query together so a
// timeout report is emitted atomically from this test-only helper.
#[allow(clippy::too_many_lines, clippy::type_complexity)]
async fn diagnose_golden_timeout(
    pool: &sqlx::PgPool,
    repository_id: uuid::Uuid,
    run_id: runtime_types::RunId,
) {
    let run = sqlx::query(
        "SELECT state, outcome, exit_code, exit_signal,
                vm_id IS NOT NULL AS has_vm
           FROM runs WHERE id = $1",
    )
    .bind(run_id.as_uuid())
    .fetch_optional(pool)
    .await;
    match run {
        Ok(Some(row)) => {
            let state: String = row.get("state");
            let outcome: Option<String> = row.get("outcome");
            let exit_code: Option<i32> = row.get("exit_code");
            let exit_signal: Option<i32> = row.get("exit_signal");
            let has_vm: bool = row.get("has_vm");
            eprintln!(
                "golden timeout: run state={state} outcome={outcome:?} exit_code={exit_code:?} exit_signal={exit_signal:?} has_vm={has_vm}"
            );
        }
        Ok(None) => eprintln!("golden timeout: run row missing"),
        Err(error) => eprintln!("golden timeout: run query failed: {error}"),
    }

    let request = sqlx::query(
        "SELECT request_kind, dispatch_state
           FROM run_requests WHERE run_id = $1",
    )
    .bind(run_id.as_uuid())
    .fetch_optional(pool)
    .await;
    match request {
        Ok(Some(row)) => {
            let kind: String = row.get("request_kind");
            let dispatch_state: String = row.get("dispatch_state");
            eprintln!("golden timeout: run request kind={kind} dispatch_state={dispatch_state}");
        }
        Ok(None) => eprintln!("golden timeout: run request row missing"),
        Err(error) => eprintln!("golden timeout: run request query failed: {error}"),
    }

    match sqlx::query_scalar::<_, String>(
        "SELECT event_type FROM run_events WHERE run_id = $1 ORDER BY sequence",
    )
    .bind(run_id.as_uuid())
    .fetch_all(pool)
    .await
    {
        Ok(events) => eprintln!("golden timeout: run events={events:?}"),
        Err(error) => eprintln!("golden timeout: run events query failed: {error}"),
    }

    match sqlx::query(
        "SELECT subject, published_at IS NOT NULL AS published
           FROM outbox
          WHERE aggregate_id = $1
          ORDER BY occurred_at, id",
    )
    .bind(run_id.as_uuid())
    .fetch_all(pool)
    .await
    {
        Ok(rows) => {
            let entries: Vec<(String, bool)> = rows
                .into_iter()
                .map(|row| (row.get("subject"), row.get("published")))
                .collect();
            eprintln!("golden timeout: run outbox={entries:?}");
        }
        Err(error) => eprintln!("golden timeout: run outbox query failed: {error}"),
    }

    match sqlx::query(
        "SELECT request.state AS request_state,
                execution.state AS execution_state,
                execution.exit_code, execution.exit_signal,
                execution.failure_code
           FROM build_requests AS request
           LEFT JOIN build_executions AS execution
             ON execution.build_request_id = request.id
          WHERE request.repository_id = $1
          ORDER BY request.created_at DESC, request.id DESC
          LIMIT 3",
    )
    .bind(repository_id)
    .fetch_all(pool)
    .await
    {
        Ok(rows) => {
            let entries: Vec<(
                String,
                Option<String>,
                Option<i32>,
                Option<i32>,
                Option<String>,
            )> = rows
                .into_iter()
                .map(|row| {
                    (
                        row.get("request_state"),
                        row.get("execution_state"),
                        row.get("exit_code"),
                        row.get("exit_signal"),
                        row.get("failure_code"),
                    )
                })
                .collect();
            eprintln!("golden timeout: recent builds={entries:?}");
        }
        Err(error) => eprintln!("golden timeout: build query failed: {error}"),
    }
}

async fn git_backend() -> PathBuf {
    let exec_path = git_output(Path::new("."), &["--exec-path"]).await;
    PathBuf::from(exec_path).join("git-http-backend")
}

async fn git_binary() -> PathBuf {
    let output = Command::new("sh")
        .args(["-c", "command -v git"])
        .output()
        .await
        .expect("resolve Git binary");
    assert!(output.status.success());
    PathBuf::from(
        String::from_utf8(output.stdout)
            .expect("UTF-8 Git path")
            .trim(),
    )
}

async fn git(directory: &Path, arguments: &[&str]) {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(directory)
        .output()
        .await
        .expect("run Git");
    assert!(
        output.status.success(),
        "Git failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

async fn authenticated_git(directory: &Path, token: &str, arguments: &[&str]) {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(directory)
        .env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "http.extraHeader")
        .env(
            "GIT_CONFIG_VALUE_0",
            format!("Authorization: Bearer {token}"),
        )
        .output()
        .await
        .expect("run authenticated Git");
    assert!(
        output.status.success(),
        "authenticated Git failed: {} {}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
}

async fn git_output(directory: &Path, arguments: &[&str]) -> String {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(directory)
        .output()
        .await
        .expect("run Git");
    assert!(output.status.success());
    String::from_utf8(output.stdout)
        .expect("UTF-8 Git output")
        .trim()
        .to_owned()
}

async fn git_output_bare(repository: &Path, arguments: &[&str]) -> String {
    let output = Command::new("git")
        .arg(format!("--git-dir={}", repository.display()))
        .args(arguments)
        .output()
        .await
        .expect("run bare Git");
    assert!(
        output.status.success(),
        "bare Git failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("UTF-8 bare Git output")
        .trim()
        .to_owned()
}

fn mkfs_ext4() -> PathBuf {
    if let Some(path) = env::var_os("HEPHAESTUS_MKFS_EXT4") {
        return PathBuf::from(path);
    }
    ["/usr/sbin/mkfs.ext4", "/usr/bin/mkfs.ext4"]
        .into_iter()
        .map(PathBuf::from)
        .find(|path| path.is_file())
        .expect("mkfs.ext4 must be installed for the golden volume fixture")
}

async fn cleanup_streams(nats_url: &str) {
    let Ok(client) = async_nats::connect(nats_url).await else {
        return;
    };
    let context = async_nats::jetstream::new(client);
    for stream in [
        "HEPH_RUN_COMMANDS",
        "HEPH_RUN_EVENTS",
        "HEPHAESTUS_GIT_EVENTS",
        "HEPHAESTUS_RELEASE_EVENTS",
        "HEPHAESTUS_PRODUCT_EVENTS",
    ] {
        drop(context.delete_stream(stream).await);
    }
}
