use hephaestus_app::{AppConfig, OidcConfig, RegistryConfig, VmBackendConfig};
use jsonwebtoken::{Algorithm, DecodingKey};
use run_runtime_local::LocalRunRuntimeConfig;
use secret_runtime::EphemeralSecretConfig;
use std::{
    env, error::Error, ffi::OsString, net::SocketAddr, path::PathBuf, sync::Arc, time::Duration,
};
use vm_libkrun::LibkrunConfig;
use volume_local::LocalVolumeConfig;
use workspace_local::{LocalWorkspaceConfig, WorkspaceLimits};
use zeroize::Zeroizing;

use super::{
    root_images::{repository_root_images, root_filesystem_path, root_images_from_environment},
    secrets::{append_repository_image_mount_roots, load_secret_keys},
    support::{
        broker_adapter_from_environment, fixed_key, gateway_edge_from_environment,
        oci_builder_from_environment, optional_u64, path, path_or, required,
    },
};

// One explicit environment-to-config mapping keeps required production
// security settings visible in a single operator-facing location.
#[allow(clippy::too_many_lines)]
pub fn environment_config() -> Result<AppConfig, Box<dyn Error>> {
    let repository_root = path("HEPHAESTUS_REPOSITORY_ROOT")?;
    let volume_root = path("HEPHAESTUS_VOLUME_ROOT")?;
    let workspace_root = path("HEPHAESTUS_WORKSPACE_ROOT")?;
    let artifact_root = path("HEPHAESTUS_ARTIFACT_ROOT")?;
    let backend_name =
        env::var("HEPHAESTUS_VM_BACKEND").unwrap_or_else(|_| String::from("libkrun"));
    let mut root_images = root_images_from_environment(&backend_name)?;
    let runtime_root = path("HEPHAESTUS_RUNTIME_ROOT")?;
    let run_runtime_root = runtime_root.join("exact-runs");
    std::fs::create_dir_all(&run_runtime_root)?;
    let oci_builder = oci_builder_from_environment(&repository_root, &runtime_root)?;
    if let Some(worker) = &oci_builder {
        for (reference, root) in repository_root_images(&worker.root_manifest, &worker.rootfs_root)?
        {
            if root_images.insert(reference.clone(), root).is_some() {
                return Err(format!(
                    "repository image root {reference:?} conflicts with a configured platform root"
                )
                .into());
            }
        }
    }
    let secret_mount_root = path("HEPHAESTUS_SECRET_RUNTIME_ROOT")?;
    let secret_broker_socket = env::var_os("HEPHAESTUS_SECRET_BROKER_SOCKET")
        .map_or_else(|| runtime_root.join("secret-broker.sock"), PathBuf::from);
    let runtime_authority_handoff_root = env::var_os("HEPHAESTUS_RUNTIME_AUTHORITY_HANDOFF_ROOT")
        .map_or_else(|| runtime_root.join("authority-handoffs"), PathBuf::from);
    let runtime_authority_handoff_key =
        fixed_key(path("HEPHAESTUS_RUNTIME_AUTHORITY_HANDOFF_KEY_FILE")?)?;
    let vm_backend = match backend_name.as_str() {
        "fake" => VmBackendConfig::Fake,
        "fixture" => VmBackendConfig::FixtureResult,
        "libkrun" => {
            let mut image_roots: Vec<_> = root_images.values().map(root_filesystem_path).collect();
            let mut mount_roots = vec![
                workspace_root.clone(),
                secret_mount_root.clone(),
                run_runtime_root.clone(),
            ];
            if let Some(worker) = &oci_builder {
                image_roots.push(worker.rootfs_root.clone());
                append_repository_image_mount_roots(
                    &mut mount_roots,
                    &worker.runtime,
                    &worker.verification_root,
                );
            }
            let mut disk_roots = vec![volume_root.clone()];
            if let Some(worker) = &oci_builder {
                disk_roots.push(worker.scratch_root.clone());
            }
            let mut config = LibkrunConfig::new(
                &runtime_root,
                image_roots,
                disk_roots,
                mount_roots,
                path("HEPHAESTUS_LIBKRUN_WORKER")?,
                path("HEPHAESTUS_CGROUP_ROOT")?,
            );
            if let Ok(library) = env::var("HEPHAESTUS_LIBKRUN_LIBRARY") {
                config.libkrun_library = OsString::from(library);
            }
            VmBackendConfig::Libkrun(Box::new(config))
        }
        backend => return Err(format!("unsupported HEPHAESTUS_VM_BACKEND {backend:?}").into()),
    };
    let algorithm = match env::var("HEPHAESTUS_OIDC_ALGORITHM")
        .unwrap_or_else(|_| String::from("RS256"))
        .as_str()
    {
        "RS256" => Algorithm::RS256,
        "HS256" => Algorithm::HS256,
        algorithm => return Err(format!("unsupported OIDC algorithm {algorithm:?}").into()),
    };
    let decoding_key = match algorithm {
        Algorithm::RS256 => {
            DecodingKey::from_rsa_pem(&std::fs::read(path("HEPHAESTUS_OIDC_PUBLIC_KEY")?)?)?
        }
        Algorithm::HS256 => {
            DecodingKey::from_secret(required("HEPHAESTUS_OIDC_HS256_SECRET")?.as_bytes())
        }
        _ => return Err(String::from("unsupported configured OIDC algorithm").into()),
    };
    let http_listen = required("HEPHAESTUS_HTTP_LISTEN")?.parse::<SocketAddr>()?;
    let gateway_edge = gateway_edge_from_environment()?;
    let secret_keys = load_secret_keys(
        &path("HEPHAESTUS_SECRET_KEY_DIRECTORY")?,
        required("HEPHAESTUS_SECRET_KEY_REFERENCE")?,
    )?;
    let rpc_mediator_secret = required("HEPHAESTUS_RPC_MEDIATOR_SECRET")?;
    let registry_private_key = Zeroizing::new(std::fs::read(path(
        "HEPHAESTUS_REGISTRY_TOKEN_PRIVATE_KEY",
    )?)?);
    let registry_authority_text = required("HEPHAESTUS_REGISTRY_SERVICE")?;
    let registry_authority =
        registry_domain::RegistryAuthority::parse(registry_authority_text.clone())?;
    let registry_token_issuer = registry_token::RegistryTokenIssuer::new(
        required("HEPHAESTUS_REGISTRY_TOKEN_ISSUER")?.parse()?,
        registry_authority_text.parse()?,
        registry_token::SigningKey::rs256_pem(
            required("HEPHAESTUS_REGISTRY_TOKEN_KEY_ID")?.parse()?,
            &registry_private_key,
        )?,
        registry_token::TokenLifetime::new(optional_u64(
            "HEPHAESTUS_REGISTRY_TOKEN_LIFETIME_SECONDS",
            300,
        )?)?,
    );
    let registry_notification_callback = Zeroizing::new(
        std::fs::read_to_string(path(
            "HEPHAESTUS_REGISTRY_NOTIFICATION_CALLBACK_TOKEN_FILE",
        )?)?
        .trim()
        .to_owned(),
    );
    Ok(AppConfig {
        database_url: required("HEPHAESTUS_DATABASE_URL")?,
        nats_url: required("HEPHAESTUS_NATS_URL")?,
        http_listen,
        rpc_mediator_signing_key: hephaestus_app::rpc::mediator_signing_key(
            rpc_mediator_secret.as_bytes(),
        ),
        repository_root: repository_root.clone(),
        git_http_backend: path("HEPHAESTUS_GIT_HTTP_BACKEND")?,
        git_pre_receive_hook: path("HEPHAESTUS_GIT_PRE_RECEIVE_HOOK")?,
        git_http_limits: git_http::GitHttpLimits::default(),
        oidc: OidcConfig {
            issuer: required("HEPHAESTUS_OIDC_ISSUER")?,
            audience: required("HEPHAESTUS_OIDC_AUDIENCE")?,
            algorithm,
            decoding_key,
        },
        registry: RegistryConfig {
            token_issuer: Arc::new(registry_token_issuer),
            notification_callback: registry_notification::CallbackCredential::parse(
                &*registry_notification_callback,
            )?,
            zot: registry_zot::ZotClientConfig::new(
                registry_authority,
                &required("HEPHAESTUS_REGISTRY_PRIVATE_ORIGIN")?,
            )?,
            reconciliation_lease: Duration::from_secs(optional_u64(
                "HEPHAESTUS_REGISTRY_RECONCILIATION_LEASE_SECONDS",
                30,
            )?),
            reconciliation_interval: Duration::from_millis(optional_u64(
                "HEPHAESTUS_REGISTRY_RECONCILIATION_INTERVAL_MILLISECONDS",
                5_000,
            )?),
        },
        gateway_edge,
        volumes: LocalVolumeConfig {
            volume_root,
            transient_runtime_roots: vec![
                runtime_root,
                workspace_root.clone(),
                secret_mount_root.clone(),
            ],
            host_id: required("HEPHAESTUS_HOST_ID")?,
            lease_duration: Duration::from_secs(30),
            mkfs_ext4: path_or("HEPHAESTUS_MKFS_EXT4", "/usr/bin/mkfs.ext4"),
        },
        workspaces: LocalWorkspaceConfig {
            workspace_root: workspace_root.clone(),
            artifact_root: artifact_root.clone(),
            repository_root,
            git_binary: path_or("HEPHAESTUS_GIT_BINARY", "/usr/bin/git"),
            limits: WorkspaceLimits::default(),
        },
        run_runtime: LocalRunRuntimeConfig {
            runtime_root: run_runtime_root,
            release_artifact_root: artifact_root.join("releases"),
        },
        runtime_authority_handoff_root,
        runtime_authority_handoff_key,
        runtime_authority_session_ttl: Duration::from_secs(optional_u64(
            "HEPHAESTUS_RUNTIME_AUTHORITY_SESSION_TTL_SECONDS",
            3_600,
        )?),
        build_workspace_root: workspace_root.join("isolated-builds"),
        build_timeout: Duration::from_secs(15 * 60),
        secret_mounts: EphemeralSecretConfig {
            root: secret_mount_root,
            require_memory_filesystem: true,
        },
        secret_keys,
        secret_broker_socket,
        secret_broker_adapter: broker_adapter_from_environment()?,
        vm_backend,
        root_images,
        oci_builder,
        runtime_policy: hephaestus_app::RuntimePolicy {
            version: required("HEPHAESTUS_RUNTIME_POLICY_VERSION")?,
            max_vcpus: required("HEPHAESTUS_RUNTIME_MAX_VCPUS")?.parse()?,
            max_memory_mib: required("HEPHAESTUS_RUNTIME_MAX_MEMORY_MIB")?.parse()?,
            allow_broker_only: required("HEPHAESTUS_RUNTIME_ALLOW_BROKER_ONLY")?.parse()?,
            allow_egress: required("HEPHAESTUS_RUNTIME_ALLOW_EGRESS")?.parse()?,
        },
        agent_state_capacity_bytes: 64 * 1024 * 1024,
        worker_concurrency: 64,
        outbox_poll_interval: Duration::from_millis(50),
        outbox_batch_size: 100,
        startup_timeout: Duration::from_secs(30),
        shutdown_timeout: Duration::from_secs(30),
    })
}
