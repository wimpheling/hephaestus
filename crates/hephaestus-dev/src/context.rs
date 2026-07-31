use crate::process::{DevError, Result, output};
use std::{
    env,
    path::{Path, PathBuf},
};

pub const POSTGRES_CONTAINER: &str = "hephaestus-local-postgres";
pub const NATS_CONTAINER: &str = "hephaestus-local-nats";
pub const WEB_CONTAINER: &str = "hephaestus-local-web";
pub const ROOTFS_CONTAINER: &str = "hephaestus-local-rootfs";
pub const POSTGRES_VOLUME: &str = "hephaestus-local-postgres-data";
pub const NATS_VOLUME: &str = "hephaestus-local-nats-data";
pub const POSTGRES_IMAGE: &str = "docker.io/library/postgres:17-alpine";
pub const NATS_IMAGE: &str = "docker.io/library/nats:2.11-alpine";
pub const ELIXIR_IMAGE: &str =
    "docker.io/hexpm/elixir:1.18.4-erlang-27.3.4-debian-bookworm-20250428-slim";
pub const FEDORA_IMAGE: &str = "registry.fedoraproject.org/fedora-minimal@sha256:8f42d200f04990b41081322d1c260ddf23b124b3b92538665ef4cc3064537249";
pub const ROOT_IMAGE_DIRECTORY: &str = "fedora-minimal-8f42d200";
pub const GUEST_TARGET: &str = "x86_64-unknown-linux-musl";

#[derive(Clone, Debug)]
// Explicit root suffixes make destructive filesystem targets unambiguous.
#[allow(clippy::struct_field_names)]
pub struct DevContext {
    pub repository_root: PathBuf,
    pub local_root: PathBuf,
    pub runtime_root: PathBuf,
    pub secret_runtime_root: PathBuf,
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
        Ok(Self {
            repository_root,
            local_root,
            runtime_root,
            secret_runtime_root,
        })
    }

    pub fn root_image(&self) -> PathBuf {
        self.local_root
            .join("root-images")
            .join(ROOT_IMAGE_DIRECTORY)
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
}
