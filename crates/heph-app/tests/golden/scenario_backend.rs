use super::*;

/// Backend, observer, and application configuration for one golden daemon.
pub struct GoldenBackendSetup {
    pub root_image: PathBuf,
    pub observer: Option<Arc<support::vm_observer::VmSpecObserver>>,
    pub app_config: AppConfig,
}

#[allow(
    clippy::fn_params_excessive_bools,
    clippy::needless_borrow,
    clippy::ref_option,
    clippy::too_many_arguments,
    clippy::too_many_lines
)]
pub async fn configure_golden_backend(
    pool: &sqlx::PgPool,
    root: &Path,
    repository_root: &Path,
    workload_phase_timing: bool,
    release_build_proof: bool,
    libkrun_e2e: bool,
    session_chat_e2e: bool,
    database_url: &str,
    nats_url: &str,
    browser_oidc_issuer: &str,
    initial_gateway_edge: Option<GatewayEdgeConfig>,
    brokered_fixture: &Option<BrokeredFixture>,
    session_chat_fixture: &Option<session_chat::SessionBrokerFixture>,
    build_timeout: Duration,
) -> GoldenBackendSetup {
    let mut backend_fixture = backend_fixture(&root).await;
    let root_image = backend_fixture.root_image.clone();
    let mut root_images = BTreeMap::from([(
        String::from(ROOT_IMAGE),
        RootFilesystem::Directory {
            host_path: root_image.clone(),
        },
    )]);
    if release_build_proof {
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
            .execute(pool)
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
    let cooking_worker = if release_build_proof {
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
        if release_build_proof {
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
    let observer = if release_build_proof {
        assert!(
            libkrun_e2e,
            "cooking build proof requires the real libkrun backend"
        );
        let required_image_roots = cooking_worker
            .as_ref()
            .map(|worker| vec![worker.rootfs_root.clone()])
            .unwrap_or_default();
        let mut credential_patterns = cooking_confinement::credential_patterns();
        if session_chat_e2e {
            credential_patterns.push(session_chat::MODEL_SECRET_VALUE.as_bytes().to_vec());
        }
        Some(
            backend_fixture
                .install_vm_observer(
                    credential_patterns,
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
    let git_pre_receive_hook = if session_chat_e2e {
        let hook = env::var_os("HEPHAESTUS_GIT_PRE_RECEIVE_HOOK")
            .map(PathBuf::from)
            .expect("session-chat requires HEPHAESTUS_GIT_PRE_RECEIVE_HOOK");
        assert!(
            hook.is_absolute() && hook.is_file(),
            "session-chat receive hook must be an absolute built executable: {}",
            hook.display()
        );
        hook
    } else {
        let hook = root.join("git-hooks/pre-receive");
        std::fs::create_dir_all(hook.parent().expect("Git hook has a parent directory"))
            .expect("Git hook directory");
        std::fs::write(&hook, b"#!/bin/sh\nexit 1\n").expect("write fail-closed Git hook fixture");
        std::fs::set_permissions(&hook, std::os::unix::fs::PermissionsExt::from_mode(0o700))
            .expect("Git hook fixture mode");
        hook
    };
    let secret_mount_root = root.join("secret-mounts");
    std::fs::create_dir(&secret_mount_root).expect("secret mount root");
    std::fs::set_permissions(
        &secret_mount_root,
        std::os::unix::fs::PermissionsExt::from_mode(0o700),
    )
    .expect("secret mount root mode");
    let app_config = AppConfig {
        database_url: database_url.to_owned(),
        nats_url: nats_url.to_owned(),
        http_listen: "127.0.0.1:0".parse().expect("ephemeral listen address"),
        rpc_mediator_signing_key: hephaestus_app::rpc::mediator_signing_key(
            b"golden-internal-command-token-with-sufficient-entropy",
        ),
        repository_root: repository_root.to_path_buf(),
        git_http_backend: backend,
        git_pre_receive_hook,
        git_http_limits: git_http::GitHttpLimits::default(),
        oidc: OidcConfig {
            issuer: browser_oidc_issuer.to_owned(),
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
            repository_root: repository_root.to_path_buf(),
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
        secret_broker_adapter: session_chat_fixture.as_ref().map_or_else(
            || {
                brokered_fixture.as_ref().map_or_else(
                    || Arc::new(DenyingBrokerAdapter) as Arc<dyn secret_application::BrokerAdapter>,
                    |fixture| fixture.upstream.adapter(),
                )
            },
            |fixture| fixture.adapter(),
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
    GoldenBackendSetup {
        root_image,
        observer,
        app_config,
    }
}
