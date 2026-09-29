use super::*;

pub async fn wait_for_gateway_invocation_completed(pool: &sqlx::PgPool, invocation_id: uuid::Uuid) {
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

pub struct ExternalGoldenDaemon {
    pub child: tokio::process::Child,
    pub base_url: String,
    pub log_path: PathBuf,
}

impl Drop for ExternalGoldenDaemon {
    fn drop(&mut self) {
        if self.child.id().is_some() {
            let _ = self.child.start_kill();
        }
    }
}

impl ExternalGoldenDaemon {
    pub async fn graceful_shutdown(mut self) {
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

    pub async fn unclean_kill(mut self) -> std::process::ExitStatus {
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
pub async fn spawn_external_golden_daemon(
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
