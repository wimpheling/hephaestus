use crate::process::{DevError, Result, output};
use std::{
    env,
    path::{Path, PathBuf},
};

pub const POSTGRES_IMAGE: &str = "docker.io/library/postgres:17-alpine";
pub const NATS_IMAGE: &str = "docker.io/library/nats:2.11-alpine";
pub const ELIXIR_IMAGE: &str =
    "docker.io/hexpm/elixir:1.18.4-erlang-27.3.4-debian-bookworm-20250428-slim";
/// The development environment has one general OCI-image cache. Additional
/// immutable images may be supplied through `HEPHAESTUS_LOCAL_OCI_IMAGES`.
pub const DEFAULT_LOCAL_OCI_IMAGE: &str = "docker.io/library/ubuntu@sha256:52df9b1ee71626e0088f7d400d5c6b5f7bb916f8f0c82b474289a4ece6cf3faf";
pub const GUEST_TARGET: &str = "x86_64-unknown-linux-musl";
/// Immutable Zot v2.1.18 OCI index used by the forge deployment contract.
pub const ZOT_IMAGE: &str = "ghcr.io/project-zot/zot@sha256:6f7bf2b8e43437c7c3a121bc80214845c85f27321e66f2ff4be6bf4220775fd7";
pub const ZOT_STORAGE_ROOT: &str = "/var/lib/registry";
pub const LOCAL_ZOT_TOKEN_REALM: &str = "http://127.0.0.1:8080/v1/registry/token";
pub const LOCAL_ZOT_NOTIFICATION_SINK: &str =
    "http://127.0.0.1:8080/internal/v1/registry/notifications";

#[derive(Clone, Debug)]
// Explicit root suffixes make destructive filesystem targets unambiguous.
#[allow(clippy::struct_field_names)]
pub struct DevContext {
    pub repository_root: PathBuf,
    pub local_root: PathBuf,
    pub runtime_root: PathBuf,
    pub secret_runtime_root: PathBuf,
    pub namespace: String,
    pub postgres_port: u16,
    pub zot_port: u16,
}

impl DevContext {
    pub fn discover() -> Result<Self> {
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
        let repository_root = manifest
            .parent()
            .and_then(Path::parent)
            .ok_or_else(|| DevError::Invalid("development crate is outside the workspace".into()))?
            .canonicalize()?;
        let local_root = env::var_os("HEPHAESTUS_LOCAL_ROOT")
            .map_or_else(|| repository_root.join(".local/hephaestus"), PathBuf::from);
        let uid = output("id", &["-u"])?;
        let runtime_root = env::var_os("HEPHAESTUS_LOCAL_RUNTIME_ROOT").map_or_else(
            || PathBuf::from(format!("/tmp/hephaestus-runtime-{uid}")),
            PathBuf::from,
        );
        let secret_runtime_root = env::var_os("HEPHAESTUS_LOCAL_SECRET_RUNTIME_ROOT").map_or_else(
            || PathBuf::from(format!("/dev/shm/hephaestus-secret-runtime-{uid}")),
            PathBuf::from,
        );
        let namespace = env::var("HEPHAESTUS_LOCAL_NAMESPACE")
            .unwrap_or_else(|_| String::from("hephaestus-local"));
        if namespace.is_empty()
            || !namespace
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        {
            return Err(DevError::Invalid(
                "HEPHAESTUS_LOCAL_NAMESPACE must contain lowercase letters, digits, and hyphens"
                    .into(),
            ));
        }
        let postgres_port = env::var("HEPHAESTUS_LOCAL_POSTGRES_PORT")
            .map_or(Ok(55432), |value| value.parse::<u16>())
            .map_err(|_| {
                DevError::Invalid("HEPHAESTUS_LOCAL_POSTGRES_PORT must be a valid TCP port".into())
            })?;
        if postgres_port < 1024 {
            return Err(DevError::Invalid(
                "HEPHAESTUS_LOCAL_POSTGRES_PORT must be between 1024 and 65535".into(),
            ));
        }
        let zot_port = env::var("HEPHAESTUS_LOCAL_ZOT_PORT")
            .map_or(Ok(55000), |value| value.parse::<u16>())
            .map_err(|_| {
                DevError::Invalid("HEPHAESTUS_LOCAL_ZOT_PORT must be a valid TCP port".into())
            })?;
        if zot_port < 1024 || zot_port == postgres_port {
            return Err(DevError::Invalid(
                "HEPHAESTUS_LOCAL_ZOT_PORT must be between 1024 and 65535 and differ from the PostgreSQL port".into(),
            ));
        }
        Ok(Self {
            repository_root,
            local_root,
            runtime_root,
            secret_runtime_root,
            namespace,
            postgres_port,
            zot_port,
        })
    }

    pub fn postgres_container(&self) -> String {
        format!("{}-postgres", self.namespace)
    }

    pub fn nats_container(&self) -> String {
        format!("{}-nats", self.namespace)
    }

    pub fn web_container(&self) -> String {
        format!("{}-web", self.namespace)
    }

    pub fn image_container(&self, digest: &str) -> String {
        let prefix = digest.chars().take(12).collect::<String>();
        format!("{}-image-{prefix}", self.namespace)
    }

    pub fn zot_container(&self) -> String {
        format!("{}-zot", self.namespace)
    }

    pub fn postgres_volume(&self) -> String {
        format!("{}-postgres-data", self.namespace)
    }

    pub fn nats_volume(&self) -> String {
        format!("{}-nats-data", self.namespace)
    }

    /// Private daemon-owned cache of materialized immutable OCI images.
    pub fn image_cache(&self) -> PathBuf {
        self.local_root.join("oci-images")
    }

    /// Atomically rewritten immutable-reference-to-local-rootfs index.
    pub fn image_manifest(&self) -> PathBuf {
        self.image_cache().join("manifest.json")
    }

    /// Private release evidence and OCI layouts created by explicit platform
    /// image operations. This is intentionally separate from VM root files.
    pub fn platform_image_releases(&self) -> PathBuf {
        self.local_root.join("platform-images/releases")
    }

    /// Durable records produced only after a reviewed local image publication.
    pub fn platform_image_installations(&self) -> PathBuf {
        self.local_root.join("platform-images/installations")
    }

    pub fn platform_image_tool_storage_volume(&self) -> String {
        format!("{}-platform-image-tool-storage", self.namespace)
    }

    pub fn platform_image_tool_cache_volume(&self) -> String {
        format!("{}-platform-image-tool-cache", self.namespace)
    }

    pub fn seed_file(&self) -> PathBuf {
        self.local_root.join("seed.json")
    }

    pub fn supervisor_pid_file(&self) -> PathBuf {
        self.local_root.join("run-local.pid")
    }

    pub fn logs(&self) -> PathBuf {
        self.local_root.join("logs")
    }

    /// The registry state is intentionally separate from repository, VM, and
    /// general-purpose local state so it can be inspected and cleaned safely.
    pub fn zot_root(&self) -> PathBuf {
        self.local_root.join("zot")
    }

    pub fn zot_storage(&self) -> PathBuf {
        self.zot_root().join("storage")
    }

    pub fn zot_config(&self) -> PathBuf {
        self.zot_root().join("config.json")
    }

    pub fn zot_verification_certificate(&self) -> PathBuf {
        self.zot_root().join("verification.crt")
    }

    pub fn zot_signing_key(&self) -> PathBuf {
        self.zot_root()
            .join("secrets/registry-token-signing-key.pem")
    }

    pub fn zot_notification_callback_token(&self) -> PathBuf {
        self.zot_root().join("secrets/notification-callback-token")
    }

    pub fn zot_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.zot_port)
    }

    pub fn zot_service(&self) -> String {
        format!("localhost:{}", self.zot_port)
    }

    pub const fn zot_token_realm() -> &'static str {
        LOCAL_ZOT_TOKEN_REALM
    }
}
