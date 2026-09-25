use hephaestus_app::{GatewayEdgeConfig, OciBuilderWorkerConfig, UiOriginConfig};
use oci_builder_runtime_local::LocalOciRuntimeConfig;
use release_service::{UiNamespace, UiPublicPort};
use secret_application::BrokerAdapter;
use secret_broker::{
    BrokeredHttpsAdapterRegistry, BrokeredHttpsOrigin, BrokeredHttpsUpstream, DenyingBrokerAdapter,
};
use std::{
    collections::BTreeMap,
    env,
    error::Error,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use zeroize::Zeroizing;

pub fn broker_adapter_from_environment() -> Result<Arc<dyn BrokerAdapter>, Box<dyn Error>> {
    let upstreams = env::var_os("HEPHAESTUS_BROKERED_HTTPS_UPSTREAMS_JSON")
        .map(|raw| {
            raw.into_string().map_err(|_| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "HEPHAESTUS_BROKERED_HTTPS_UPSTREAMS_JSON must be valid UTF-8",
                )
            })
        })
        .transpose()?
        .map(|raw| serde_json::from_str::<Vec<BrokeredHttpsUpstream>>(&raw))
        .transpose()?;
    let origins = env::var_os("HEPHAESTUS_BROKERED_HTTPS_ORIGIN_CATALOG_JSON")
        .map(|raw| {
            raw.into_string().map_err(|_| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "HEPHAESTUS_BROKERED_HTTPS_ORIGIN_CATALOG_JSON must be valid UTF-8",
                )
            })
        })
        .transpose()?
        .map(|raw| serde_json::from_str::<Vec<BrokeredHttpsOrigin>>(&raw))
        .transpose()?;
    if upstreams.is_none() && origins.is_none() {
        return Ok(Arc::new(DenyingBrokerAdapter));
    }
    let upstreams = upstreams.unwrap_or_default();
    let origins = origins.unwrap_or_default();
    if upstreams.is_empty() && origins.is_empty() {
        return Err("brokered HTTPS rule or origin catalog must not be empty".into());
    }
    Ok(Arc::new(
        BrokeredHttpsAdapterRegistry::new_with_origin_catalog(upstreams, origins)?,
    ))
}

pub fn gateway_edge_from_environment() -> Result<Option<GatewayEdgeConfig>, Box<dyn Error>> {
    let caddy_admin_url = optional_environment("HEPHAESTUS_CADDY_ADMIN_URL")?;
    let ui_listen = optional_environment("HEPHAESTUS_UI_ORIGIN_LISTEN")?;
    let ui_namespace = optional_environment("HEPHAESTUS_UI_NAMESPACE")?;
    let ui_port = optional_environment("HEPHAESTUS_UI_PORT")?;
    let ui_platform_origin = optional_environment("HEPHAESTUS_PLATFORM_HTTPS_ORIGIN")?;
    let ui_origin = parse_ui_origin_config(
        caddy_admin_url.is_some(),
        ui_listen.as_deref(),
        ui_namespace.as_deref(),
        ui_port.as_deref(),
        ui_platform_origin.as_deref(),
    )?;
    let Some(caddy_admin_url) = caddy_admin_url else {
        return Ok(None);
    };
    Ok(Some(GatewayEdgeConfig {
        caddy_admin_url,
        dispatcher_listen: required("HEPHAESTUS_GATEWAY_DISPATCHER_LISTEN")?.parse()?,
        public_authority: required("HEPHAESTUS_GATEWAY_PUBLIC_AUTHORITY")?,
        caddy_configuration_template: std::fs::read(path("HEPHAESTUS_CADDY_CONFIGURATION_FILE")?)?,
        caddy_server_name: required("HEPHAESTUS_CADDY_SERVER_NAME")?,
        ui_origin,
    }))
}

fn optional_environment(name: &str) -> Result<Option<String>, Box<dyn Error>> {
    match env::var_os(name) {
        Some(value) => Ok(Some(
            value
                .into_string()
                .map_err(|_| format!("{name} must be valid UTF-8"))?,
        )),
        None => Ok(None),
    }
}

pub fn parse_ui_origin_config(
    caddy_configured: bool,
    listen: Option<&str>,
    namespace: Option<&str>,
    port: Option<&str>,
    platform_origin: Option<&str>,
) -> Result<Option<UiOriginConfig>, Box<dyn Error>> {
    let enabled =
        listen.is_some() || namespace.is_some() || port.is_some() || platform_origin.is_some();
    if !enabled {
        return Ok(None);
    }
    if !caddy_configured {
        return Err("UI origin configuration requires the existing Caddy configuration".into());
    }
    let listen: SocketAddr = listen
        .ok_or("HEPHAESTUS_UI_ORIGIN_LISTEN is required when UI origin is enabled")?
        .parse()?;
    if !listen.ip().is_loopback() || listen.port() == 0 {
        return Err("HEPHAESTUS_UI_ORIGIN_LISTEN must be a nonzero loopback address".into());
    }
    let namespace = UiNamespace::parse(
        namespace.ok_or("HEPHAESTUS_UI_NAMESPACE is required when UI origin is enabled")?,
    )?;
    let port = port.map_or_else(
        || Ok(UiPublicPort::https_default()),
        UiPublicPort::parse_text,
    )?;
    let platform_origin = platform_origin
        .ok_or("HEPHAESTUS_PLATFORM_HTTPS_ORIGIN is required when UI origin is enabled")?;
    Ok(Some(
        UiOriginConfig::new(namespace, port, platform_origin)?.with_listener(listen),
    ))
}

pub fn fixed_key(path: PathBuf) -> Result<[u8; 32], Box<dyn Error>> {
    let bytes = Zeroizing::new(std::fs::read(path)?);
    bytes
        .as_slice()
        .try_into()
        .map_err(|_| "runtime authority handoff key must contain exactly 32 bytes".into())
}

pub fn required(name: &str) -> Result<String, Box<dyn Error>> {
    env::var(name).map_err(|_| format!("{name} is required").into())
}

pub fn path(name: &str) -> Result<PathBuf, Box<dyn Error>> {
    Ok(PathBuf::from(required(name)?))
}

pub fn oci_builder_from_environment(
    repository_root: &Path,
    runtime_root: &Path,
) -> Result<Option<OciBuilderWorkerConfig>, Box<dyn Error>> {
    let Some(rootfs_root) = env::var_os("HEPHAESTUS_OCI_BUILDER_ROOTFS_ROOT") else {
        return Ok(None);
    };
    let base_layout_manifest = path("HEPHAESTUS_OCI_BUILDER_BASE_LAYOUT_MANIFEST")?;
    if !base_layout_manifest.is_absolute() {
        return Err(String::from("OCI base-layout manifest path must be absolute").into());
    }
    let image_layouts: BTreeMap<String, PathBuf> =
        serde_json::from_slice(&std::fs::read(base_layout_manifest)?)?;
    if image_layouts.is_empty() {
        return Err(String::from("OCI base-layout manifest must not be empty").into());
    }
    let host_id = required("HEPHAESTUS_HOST_ID")?;
    let preparation_worker_name = env::var("HEPHAESTUS_OCI_BUILDER_PREPARATION_WORKER")
        .unwrap_or_else(|_| format!("oci-preparation-{host_id}"));
    let materialization_worker_name = env::var("HEPHAESTUS_OCI_BUILDER_MATERIALIZATION_WORKER")
        .unwrap_or_else(|_| format!("oci-rootfs-{host_id}"));
    let output_root = path("HEPHAESTUS_OCI_BUILDER_OUTPUT_ROOT")?;
    let verification_root = path("HEPHAESTUS_OCI_BUILDER_VERIFICATION_ROOT")?;
    let registry_authority =
        registry_domain::RegistryAuthority::parse(required("HEPHAESTUS_REGISTRY_SERVICE")?)?;
    let runtime = LocalOciRuntimeConfig {
        repository_root: repository_root.to_path_buf(),
        checkout_root: path("HEPHAESTUS_OCI_BUILDER_CHECKOUT_ROOT")?,
        image_layouts,
        output_root: output_root.clone(),
        verified_rootfs_root: Some(verification_root.clone()),
        git_binary: path_or("HEPHAESTUS_GIT_BINARY", "/usr/bin/git"),
        tar_binary: path_or("HEPHAESTUS_TAR_BINARY", "/usr/bin/tar"),
        buildah_binary: None,
        trivy_binary: None,
        umoci_binary: env::var_os("HEPHAESTUS_UMOCI_BINARY").map(PathBuf::from),
        buildah_output_prefix: env::var("HEPHAESTUS_OCI_BUILDER_OUTPUT_PREFIX")
            .unwrap_or_else(|_| String::from("heph-builder")),
    };
    let workload_phase_timing = env::var_os("HEPH_GCP_PHASE_TIMING_PATH").is_some();
    let publisher = registry_publisher::PublisherConfiguration::new(
        registry_authority,
        &output_root,
        &verification_root,
        &path("HEPHAESTUS_REGISTRY_CREDENTIAL_ROOT")?,
        &path_or("HEPHAESTUS_SKOPEO", "/usr/bin/skopeo"),
        &path_or("HEPHAESTUS_ORAS", "/usr/bin/oras"),
    )?;
    // A public registry uses the HTTPS origin derived from its authority. The
    // local development Zot endpoint is deliberately an explicit HTTP-only
    // override, shared with the reconciliation and platform-image tooling.
    let publisher = match env::var("HEPHAESTUS_REGISTRY_PRIVATE_ORIGIN") {
        Ok(origin) => publisher.with_registry_origin(&origin)?,
        Err(env::VarError::NotPresent) => publisher,
        Err(error) => return Err(Box::new(error)),
    };
    Ok(Some(OciBuilderWorkerConfig {
        runtime,
        publisher,
        publication_policy_version: registry_domain::PolicyVersion::parse(
            env::var("HEPHAESTUS_REGISTRY_POLICY_VERSION")
                .unwrap_or_else(|_| String::from("registry/v1")),
        )?,
        publication_policy: registry_domain::SupplyChainPolicy::without_signature(),
        builder_vm_image: builder_catalog_domain::OciImageReference::parse(required(
            "HEPHAESTUS_OCI_BUILDER_VM_IMAGE",
        )?)?,
        verifier_vm_image: builder_catalog_domain::OciImageReference::parse(required(
            "HEPHAESTUS_OCI_VERIFIER_VM_IMAGE",
        )?)?,
        verification_root,
        scratch_root: path("HEPHAESTUS_OCI_BUILDER_SCRATCH_ROOT")?,
        mkfs_ext4: path_or("HEPHAESTUS_MKFS_EXT4", "/usr/sbin/mkfs.ext4"),
        vm_resources: vm_trait::VmResources {
            vcpus: optional_u64("HEPHAESTUS_OCI_BUILDER_VM_VCPUS", 1)?.try_into()?,
            memory_mib: optional_u64("HEPHAESTUS_OCI_BUILDER_VM_MEMORY_MIB", 1024)?.try_into()?,
        },
        workload_phase_timing,
        preparation_worker_name,
        materialization_worker_name,
        rootfs_root: PathBuf::from(rootfs_root),
        root_manifest: env::var_os("HEPHAESTUS_OCI_BUILDER_ROOT_MANIFEST").map_or_else(
            || runtime_root.join("repository-builder-roots.json"),
            PathBuf::from,
        ),
        guest_init: path("HEPHAESTUS_GUEST_INIT_BINARY")?,
        lease: Duration::from_secs(optional_u64("HEPHAESTUS_OCI_BUILDER_LEASE_SECONDS", 900)?),
        poll_interval: Duration::from_millis(optional_u64(
            "HEPHAESTUS_OCI_BUILDER_POLL_MILLISECONDS",
            1_000,
        )?),
    }))
}

pub fn path_or(name: &str, fallback: &str) -> PathBuf {
    env::var_os(name).map_or_else(|| PathBuf::from(fallback), PathBuf::from)
}

pub fn optional_u64(name: &str, default: u64) -> Result<u64, Box<dyn Error>> {
    match env::var(name) {
        Ok(value) => Ok(value.parse()?),
        Err(env::VarError::NotPresent) => Ok(default),
        Err(error) => Err(Box::new(error)),
    }
}
