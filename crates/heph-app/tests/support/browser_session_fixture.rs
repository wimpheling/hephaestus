#![cfg(feature = "test-fixtures")]

//! Disposable application fixture for browser-session transport acceptance.

use hephaestus_app::{AppConfig, OidcConfig, RegistryConfig, RuntimePolicy, VmBackendConfig};
use jsonwebtoken::Algorithm;
use registry_domain::RegistryAuthority;
use registry_notification::CallbackCredential;
use registry_token::{RegistryTokenIssuer, SigningKey, TokenLifetime};
use secret_broker::DenyingBrokerAdapter;
use secret_runtime::EphemeralSecretConfig;
use secret_store::LocalKeyProvider;
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use url::Url;
use vm_trait::RootFilesystem;
use volume_local::LocalVolumeConfig;
use workspace_local::{LocalWorkspaceConfig, WorkspaceLimits};

const TEST_SIGNING_SECRET: &[u8] = b"service-log-retention-test-signing-secret";
const TEST_ROOT_IMAGE: &str = "service-log-retention-test@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn test_registry() -> RegistryConfig {
    RegistryConfig {
        token_issuer: Arc::new(RegistryTokenIssuer::new(
            "https://registry.invalid/token"
                .parse()
                .expect("registry issuer URL"),
            "registry.invalid"
                .parse()
                .expect("registry service authority"),
            SigningKey::hs256(
                "service-log-test".parse().expect("registry key id"),
                TEST_SIGNING_SECRET,
            )
            .expect("registry signing key"),
            TokenLifetime::new(300).expect("registry token lifetime"),
        )),
        notification_callback: CallbackCredential::parse(
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        )
        .expect("registry notification callback"),
        zot: registry_zot::ZotClientConfig::new(
            RegistryAuthority::parse("registry.invalid").expect("registry service"),
            "http://127.0.0.1:1/",
        )
        .expect("registry Zot configuration"),
        reconciliation_lease: Duration::from_secs(30),
        reconciliation_interval: Duration::from_secs(30),
    }
}

fn executable_git_http_backend() -> PathBuf {
    [
        PathBuf::from("/usr/lib/git-core/git-http-backend"),
        PathBuf::from("/usr/libexec/git-core/git-http-backend"),
    ]
    .into_iter()
    .find(|path| path.is_file())
    .expect("git-http-backend must be installed")
}

// Keep the disposable application fixture in one place so every path uses
// the same isolated roots and production configuration boundary.
#[allow(clippy::too_many_lines)]
pub fn app_config(database_url: String, nats_url: String, root: &Path) -> AppConfig {
    let repository_root = root.join("repositories");
    let volume_root = root.join("volumes");
    let workspace_root = root.join("workspaces");
    let artifact_root = root.join("artifacts");
    let runtime_root = root.join("runtime");
    let release_artifact_root = root.join("release-artifacts");
    let handoff_root = root.join("handoffs");
    let build_root = root.join("builds");
    let secret_mount_root = root.join("secret-mounts");
    let root_image = root.join("root-image");
    let broker_socket = root.join("secret-broker.sock");
    let hook = root.join("git-hooks/pre-receive");
    for path in [
        &repository_root,
        &volume_root,
        &workspace_root,
        &artifact_root,
        &runtime_root,
        &release_artifact_root,
        &handoff_root,
        &build_root,
        &secret_mount_root,
        &root_image,
    ] {
        fs::create_dir_all(path).expect("create application test root");
    }
    fs::set_permissions(&secret_mount_root, fs::Permissions::from_mode(0o700))
        .expect("set secret mount root mode");
    fs::create_dir_all(hook.parent().expect("hook parent")).expect("create hook root");
    fs::write(&hook, b"#!/bin/sh\nexit 1\n").expect("write fail-closed test hook");
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o700)).expect("set test hook mode");

    let mut root_images = BTreeMap::new();
    root_images.insert(
        TEST_ROOT_IMAGE.to_owned(),
        RootFilesystem::Directory {
            host_path: root_image,
        },
    );
    AppConfig {
        database_url,
        nats_url,
        http_listen: "127.0.0.1:0".parse().expect("test HTTP address"),
        rpc_mediator_signing_key: hephaestus_app::rpc::mediator_signing_key(TEST_SIGNING_SECRET),
        repository_root: repository_root.clone(),
        git_http_backend: executable_git_http_backend(),
        git_pre_receive_hook: hook,
        git_http_limits: git_http::GitHttpLimits::default(),
        oidc: OidcConfig {
            issuer: String::from("https://issuer.service-log.invalid"),
            audience: String::from("service-log-test"),
            algorithm: Algorithm::HS256,
            decoding_key: jsonwebtoken::DecodingKey::from_secret(TEST_SIGNING_SECRET),
        },
        registry: test_registry(),
        gateway_edge: None,
        volumes: LocalVolumeConfig {
            volume_root,
            transient_runtime_roots: Vec::new(),
            host_id: String::from("service-log-test-host"),
            lease_duration: Duration::from_secs(30),
            mkfs_ext4: PathBuf::from("/usr/sbin/mkfs.ext4"),
        },
        workspaces: LocalWorkspaceConfig {
            workspace_root,
            artifact_root,
            repository_root,
            git_binary: PathBuf::from("/usr/bin/git"),
            limits: WorkspaceLimits::default(),
        },
        run_runtime: run_runtime_local::LocalRunRuntimeConfig {
            runtime_root,
            release_artifact_root,
        },
        runtime_authority_handoff_root: handoff_root,
        runtime_authority_handoff_key: [0x31; 32],
        runtime_authority_session_ttl: Duration::from_secs(3_600),
        build_workspace_root: build_root,
        build_timeout: Duration::from_secs(30),
        secret_mounts: EphemeralSecretConfig {
            root: secret_mount_root,
            require_memory_filesystem: false,
        },
        secret_keys: LocalKeyProvider::new("service-log/v1", [("service-log/v1", [7_u8; 32])])
            .expect("test secret key"),
        secret_broker_socket: broker_socket,
        secret_broker_adapter: Arc::new(DenyingBrokerAdapter),
        vm_backend: VmBackendConfig::FixtureResult,
        root_images,
        oci_builder: None,
        runtime_policy: RuntimePolicy {
            version: String::from("service-log-test/v1"),
            max_vcpus: 2,
            max_memory_mib: 1_024,
            allow_broker_only: true,
            allow_egress: false,
        },
        agent_state_capacity_bytes: 16 * 1024 * 1024,
        worker_concurrency: 2,
        outbox_poll_interval: Duration::from_millis(50),
        outbox_batch_size: 20,
        startup_timeout: Duration::from_secs(20),
        shutdown_timeout: Duration::from_secs(20),
    }
}

#[allow(clippy::too_many_lines)]
pub async fn cleanup_nats(nats_url: &str) {
    let client = async_nats::connect(nats_url)
        .await
        .expect("connect NATS cleanup client");
    let context = async_nats::jetstream::new(client);
    for stream in [
        "HEPH_RUN_COMMANDS",
        "HEPH_RUN_EVENTS",
        "HEPHAESTUS_GIT_EVENTS",
        "HEPHAESTUS_RELEASE_EVENTS",
        "HEPHAESTUS_PRODUCT_EVENTS",
    ] {
        if let Err(error) = context.delete_stream(stream).await {
            assert!(
                error.to_string().contains("stream not found"),
                "delete test NATS stream: {error}"
            );
        }
    }
}

pub fn require_disposable_nats(nats_url: &str) {
    let parsed = Url::parse(nats_url).expect("parse NATS test URL");
    assert_eq!(parsed.scheme(), "nats", "retention test requires NATS URL");
    assert!(
        matches!(parsed.host_str(), Some("127.0.0.1" | "localhost" | "::1")),
        "retention test refuses non-loopback NATS URL"
    );
}
