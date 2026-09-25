//! Local runtime configuration and trusted tooling.

use super::output::{canonical_regular_file, validate_executable};
use oci_builder_worker::OciWorkerError;
use std::{collections::BTreeMap, fs, path::PathBuf, sync::Arc};

/// Explicit absolute binaries and private roots used by the local OCI worker.
#[derive(Debug, Clone)]
pub struct LocalOciRuntimeConfig {
    /// Root containing canonical bare repositories named `<uuid>.git`.
    pub repository_root: PathBuf,
    /// Private transient directory for exact source checkouts.
    pub checkout_root: PathBuf,
    /// Administrator-owned OCI layouts cached by immutable image reference.
    /// These are daemon cache entries, never repository-supplied paths.
    pub image_layouts: BTreeMap<String, PathBuf>,
    /// Private immutable OCI layout output directory.
    pub output_root: PathBuf,
    /// Optional private verifier output root. When configured, verified
    /// repository images are materialized only from a verifier-exported
    /// rootfs whose recorded digest matches the immutable image reference.
    pub verified_rootfs_root: Option<PathBuf>,
    /// Absolute trusted Git executable.
    pub git_binary: PathBuf,
    /// Absolute trusted Tar executable used only to unpack Git's exact archive.
    pub tar_binary: PathBuf,
    /// Optional legacy host Buildah executable. Production daemon composition
    /// leaves this unset and uses the isolated builder VM instead.
    pub buildah_binary: Option<PathBuf>,
    /// Optional legacy host Trivy executable. Production daemon composition
    /// leaves this unset and uses the independent verifier VM instead.
    pub trivy_binary: Option<PathBuf>,
    /// Optional trusted Umoci executable for administrator-cached base images.
    /// Repository image outputs use verifier-exported roots instead.
    pub umoci_binary: Option<PathBuf>,
    /// Administrator-owned local Buildah image-tag prefix.
    pub buildah_output_prefix: String,
}

/// Trusted tools used only after the networkless Buildah phase.
#[derive(Debug, Clone)]
pub struct ForgeZotPublicationConfig {
    /// Absolute trusted Syft executable used to produce SPDX SBOMs.
    pub syft_binary: PathBuf,
    /// Absolute, administrator-owned Syft configuration.
    pub syft_config: PathBuf,
}

impl ForgeZotPublicationConfig {
    /// Validates and canonicalizes trusted Syft tooling.
    ///
    /// # Errors
    ///
    /// Returns an error when either path is relative, missing, or symbolic.
    pub fn initialize(mut self) -> Result<Self, OciWorkerError> {
        validate_executable(&self.syft_binary)?;
        self.syft_binary =
            fs::canonicalize(&self.syft_binary).map_err(OciWorkerError::Filesystem)?;
        self.syft_config = canonical_regular_file(&self.syft_config)?;
        Ok(self)
    }
}

/// Local implementation of exact checkout, scan/publication, and rootfs export.
#[derive(Clone)]
pub struct LocalOciRuntime {
    pub(super) config: Arc<LocalOciRuntimeConfig>,
}
