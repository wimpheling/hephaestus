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
    RegistryConfig, RunEventKind, VmBackendConfig,
};
use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
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
    path::{Path, PathBuf},
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
use vm_trait::RootFilesystem;
use volume_local::LocalVolumeConfig;
use workspace_local::{LocalWorkspaceConfig, WorkspaceLimits};

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
        guest_init: PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/x86_64-unknown-linux-musl/release/heph-init"),
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
#[path = "../../../examples/cooking/tests/updates.rs"]
mod cooking_updates;
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

async fn restart_application(config: AppConfig) -> hephaestus_app::RunningHephaestus {
    HephaestusApp::build(config)
        .await
        .expect("rebuild production application after restart")
        .start()
        .await
        .expect("restart ready application")
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
    let (Ok(database_url), Ok(nats_url)) = (
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
    let libkrun_e2e = env::var("HEPHAESTUS_APP_LIBKRUN_E2E").as_deref() == Ok("1");
    let cooking_build_proof = env::var("HEPHAESTUS_APP_COOKING_BUILD_PROOF").as_deref() == Ok("1");
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
    let gateway_caddy_e2e = env::var("HEPHAESTUS_APP_GATEWAY_CADDY_E2E").as_deref() == Ok("1");
    assert!(
        !cooking::enabled() || gateway_caddy_e2e,
        "cooking requires the joined Caddy/libkrun fixture"
    );
    assert!(
        !gateway_caddy_e2e || libkrun_e2e,
        "the joined Caddy gateway proof requires the real libkrun backend"
    );
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .expect("connect golden PostgreSQL");
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("apply application migrations");

    let temporary = tempfile::tempdir().expect("golden temporary root");
    let root = temporary.path().canonicalize().expect("canonical root");
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
        &root.join("release-artifacts"),
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
    let gateway_edge = if gateway_caddy_e2e && !cooking_build_proof {
        let gateway_agent =
            seed_gateway_release_agent(&pool, &seeded_instance, &root.join("release-artifacts"))
                .await;
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
            },
            gateway_fixture,
        ))
    } else {
        None
    };

    let mut backend_fixture = backend_fixture(&root).await;
    let root_image = backend_fixture.root_image.clone();
    let mut root_images = BTreeMap::from([(
        String::from(ROOT_IMAGE),
        RootFilesystem::Directory {
            host_path: root_image,
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
        // The one-shot OCI builder formats its private scratch disk under the
        // fixture root; it must be a disk allowlist root as well as a worker
        // filesystem root.
        provider
            .disk_roots
            .push(root.join("repository-images/scratch"));
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
        gateway_edge: gateway_edge.as_ref().map(|(config, _)| config.clone()),
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
    let app = HephaestusApp::build(app_config.clone())
        .await
        .expect("build production application");
    let daemon_readiness_timer =
        WorkloadPhaseTimer::start("gateway-readiness", workload_phase_timing);
    let running = app.start().await;
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
                    "jti": uuid::Uuid::new_v4().to_string()
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
                }
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
                let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../../scripts/run-ui-e2e-external.sh");
                let browser_timer =
                    WorkloadPhaseTimer::start("browser-initial", workload_phase_timing);
                let status = tokio::process::Command::new(script)
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
                    .env("HEPHAESTUS_E2E_COOKING_PHASE", "initial")
                    .status()
                    .await;
                browser_timer.finish(status.as_ref().is_ok_and(std::process::ExitStatus::success));
                let status = status.expect("run cooking browser E2E");
                assert!(status.success(), "cooking browser E2E failed: {status}");
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
        let running = restart_application(app_config.clone()).await;
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
        let restarted = restart_application(app_config).await;
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
        let running = restart_application(app_config.clone()).await;
        let checkpoint =
            cooking::exercise_initial(&pool, &gateway_edge.as_ref().expect("cooking gateway").1)
                .await;
        running
            .shutdown()
            .await
            .expect("cooking daemon graceful restart shutdown");
        let restarted = restart_application(app_config).await;
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
        if gateway_caddy_e2e {
            let admin_url =
                env::var("HEPHAESTUS_CADDY_TEST_ADMIN_URL").expect("joined Caddy admin URL");
            let applied_config = reqwest::Client::new()
                .get(format!("{admin_url}/config/"))
                .send()
                .await
                .expect("load applied Caddy configuration")
                .error_for_status()
                .expect("Caddy configuration request succeeds")
                .text()
                .await
                .expect("read applied Caddy configuration");
            assert!(
                applied_config.contains("/gateway/brokered"),
                "Caddy must contain the authoritative gateway route before the public proof: {applied_config}"
            );
            let public_url =
                env::var("HEPHAESTUS_CADDY_TEST_PUBLIC_URL").expect("joined Caddy public URL");
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

fn caddy_configuration(admin_url: &str) -> Vec<u8> {
    let admin = reqwest::Url::parse(admin_url).expect("Caddy admin URL");
    let admin_listen = format!(
        "{}:{}",
        admin.host_str().expect("Caddy admin host"),
        admin.port().expect("Caddy admin port")
    );
    serde_json::json!({
        "admin": { "listen": admin_listen },
        "apps": { "http": { "servers": { "shared": {
            "listen": [env::var("HEPHAESTUS_CADDY_TEST_LISTEN").expect("joined Caddy listen address")],
            "routes": [
                { "match": [{ "path": ["/platform/*"] }], "handle": [{ "handler": "static_response", "body": "platform-owned" }] },
                { "group": "hephaestus.gateway", "handle": [{ "handler": "subroute", "routes": [] }] },
                { "handle": [{ "handler": "static_response", "status_code": 404 }] }
            ]
        } } } }
    })
    .to_string()
    .into_bytes()
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
