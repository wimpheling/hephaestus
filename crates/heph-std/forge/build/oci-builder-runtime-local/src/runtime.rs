//! Local runtime operations.

use super::{
    config::{ForgeZotPublicationConfig, LocalOciRuntime, LocalOciRuntimeConfig},
    constants::TRUSTED_SYSTEM_PATH,
    output::{
        canonical_directory, initialize_directory, layout_index_descriptor,
        normalize_layout_to_single_index, trusted_output_file, validate_executable,
    },
    publication::PreparedPublicationMaterial,
};
use builder_catalog_domain::OciImageReference;
use oci_builder_worker::{IsolatedOciBuild, OciWorkerError, local_image_name};
use registry_publisher::{PublicationEvidenceFiles, PublicationMaterial};
use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
};
use tokio::process::Command;

impl LocalOciRuntime {
    /// Validates all administrator-controlled binaries and roots.
    ///
    /// # Errors
    ///
    /// Returns an error for a relative, symlinked, or missing configured path.
    pub fn initialize(mut config: LocalOciRuntimeConfig) -> Result<Self, OciWorkerError> {
        validate_executable(&config.git_binary)?;
        validate_executable(&config.tar_binary)?;
        if let Some(binary) = &config.buildah_binary {
            validate_executable(binary)?;
        }
        if let Some(binary) = &config.trivy_binary {
            validate_executable(binary)?;
        }
        if let Some(binary) = &config.umoci_binary {
            validate_executable(binary)?;
        }
        config.repository_root = canonical_directory(&config.repository_root)?;
        config.checkout_root = initialize_directory(&config.checkout_root)?;
        config.output_root = initialize_directory(&config.output_root)?;
        config.verified_rootfs_root = config
            .verified_rootfs_root
            .map(|path| initialize_directory(&path))
            .transpose()?;
        if config.checkout_root.starts_with(&config.repository_root)
            || config.output_root.starts_with(&config.repository_root)
            || config.checkout_root.starts_with(&config.output_root)
            || config.output_root.starts_with(&config.checkout_root)
            || config.verified_rootfs_root.as_ref().is_some_and(|root| {
                root == &config.repository_root
                    || root.starts_with(&config.repository_root)
                    || config.repository_root.starts_with(root)
                    || root == &config.checkout_root
                    || root.starts_with(&config.checkout_root)
                    || config.checkout_root.starts_with(root)
                    || root == &config.output_root
                    || root.starts_with(&config.output_root)
                    || config.output_root.starts_with(root)
            })
            || config.buildah_output_prefix.trim().is_empty()
            || config.buildah_output_prefix.len() > 160
        {
            return Err(OciWorkerError::InvalidConfiguration);
        }
        config.image_layouts = config
            .image_layouts
            .into_iter()
            .map(|(reference, path)| {
                OciImageReference::parse(reference.clone())
                    .map_err(|_| OciWorkerError::InvalidConfiguration)?;
                Ok((reference, canonical_directory(&path)?))
            })
            .collect::<Result<_, OciWorkerError>>()?;
        Ok(Self {
            config: Arc::new(config),
        })
    }

    /// Returns the exact local Buildah image name for a job's immutable image.
    #[must_use]
    pub fn buildah_image_name(&self, request: &IsolatedOciBuild) -> String {
        local_image_name(&self.config.buildah_output_prefix, request.image_id)
    }

    /// Returns the private output root used by sealed repository layouts.
    #[must_use]
    pub fn output_root(&self) -> &Path {
        &self.config.output_root
    }

    fn legacy_buildah(&self) -> Result<&Path, OciWorkerError> {
        self.config
            .buildah_binary
            .as_deref()
            .ok_or(OciWorkerError::InvalidConfiguration)
    }

    fn legacy_trivy(&self) -> Result<&Path, OciWorkerError> {
        self.config
            .trivy_binary
            .as_deref()
            .ok_or(OciWorkerError::InvalidConfiguration)
    }

    /// Resolves the unique verifier-exported rootfs matching an image digest.
    ///
    /// # Errors
    ///
    /// Returns an error when verifier evidence is malformed or ambiguous.
    pub fn verified_rootfs_for(
        &self,
        image_reference: &OciImageReference,
    ) -> Result<Option<PathBuf>, OciWorkerError> {
        let Some(root) = &self.config.verified_rootfs_root else {
            return Ok(None);
        };
        let expected = image_reference
            .as_str()
            .rsplit_once('@')
            .map(|(_, digest)| digest)
            .ok_or(OciWorkerError::InvalidOutput)?;
        let mut matches = Vec::new();
        for entry in fs::read_dir(root).map_err(OciWorkerError::Filesystem)? {
            let entry = entry.map_err(OciWorkerError::Filesystem)?;
            let metadata =
                fs::symlink_metadata(entry.path()).map_err(OciWorkerError::Filesystem)?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(OciWorkerError::InvalidOutput);
            }
            let verification = entry.path();
            let manifest = verification.join("manifest-digest");
            let manifest_metadata = match fs::symlink_metadata(&manifest) {
                // A failed verifier leaves its job-scoped evidence root behind
                // for durable diagnostics. It never had a verified rootfs, so
                // it cannot poison a later materialization lookup.
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Ok(metadata) => metadata,
                Err(error) => return Err(OciWorkerError::Filesystem(error)),
            };
            if manifest_metadata.file_type().is_symlink() || !manifest_metadata.is_file() {
                return Err(OciWorkerError::InvalidOutput);
            }
            let digest = fs::read_to_string(&manifest).map_err(OciWorkerError::Filesystem)?;
            if digest.trim() != expected {
                continue;
            }
            let rootfs = verification.join("rootfs");
            let rootfs_metadata =
                fs::symlink_metadata(&rootfs).map_err(OciWorkerError::Filesystem)?;
            if rootfs_metadata.file_type().is_symlink() || !rootfs_metadata.is_dir() {
                return Err(OciWorkerError::InvalidOutput);
            }
            matches.push(rootfs);
        }
        match matches.len() {
            0 => Ok(None),
            1 => Ok(matches.pop()),
            _ => Err(OciWorkerError::InvalidOutput),
        }
    }

    /// Runs a trusted local tool and requires successful completion.
    ///
    /// # Errors
    ///
    /// Returns an error when the process cannot run or exits unsuccessfully.
    pub async fn command_success(
        &self,
        binary: &Path,
        arguments: Vec<OsString>,
    ) -> Result<(), OciWorkerError> {
        let status = Command::new(binary)
            .env_clear()
            // OCI tools may invoke administrator-installed helpers. Keep the
            // path fixed instead of inheriting caller-controlled state.
            .env("PATH", TRUSTED_SYSTEM_PATH)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .args(&arguments)
            .status()
            .await
            .map_err(OciWorkerError::Process)?;
        status
            .success()
            .then_some(())
            .ok_or(OciWorkerError::BuildFailed)
    }

    fn layout_directory(&self, request: &IsolatedOciBuild) -> PathBuf {
        self.config
            .output_root
            .join(request.image_id.as_uuid().to_string())
    }

    async fn publish_layout(
        &self,
        request: &IsolatedOciBuild,
        image_name: &str,
    ) -> Result<PathBuf, OciWorkerError> {
        let layout = self.layout_directory(request);
        if layout.exists() {
            fs::remove_dir_all(&layout).map_err(OciWorkerError::Filesystem)?;
        }
        fs::create_dir_all(&layout).map_err(OciWorkerError::Filesystem)?;
        self.command_success(
            self.legacy_buildah()?,
            vec![
                OsString::from("push"),
                OsString::from("--format"),
                OsString::from("oci"),
                OsString::from(image_name),
                OsString::from(format!("oci:{}:latest", layout.display())),
            ],
        )
        .await?;
        normalize_layout_to_single_index(&layout)
    }

    async fn scan_layout(
        &self,
        request: &IsolatedOciBuild,
        image_name: &str,
        layout: &Path,
    ) -> Result<PathBuf, OciWorkerError> {
        let scan = layout.join("scan.json");
        let archive = self
            .config
            .output_root
            .join(format!("{}.oci.tar", request.image_id.as_uuid()));
        if archive.exists() {
            return Err(OciWorkerError::UnsafeSourcePath);
        }
        self.command_success(
            self.legacy_buildah()?,
            vec![
                OsString::from("push"),
                OsString::from("--format"),
                OsString::from("oci"),
                OsString::from(image_name),
                OsString::from(format!("oci-archive:{}:latest", archive.display())),
            ],
        )
        .await?;
        let scan_result = self
            .command_success(
                self.legacy_trivy()?,
                vec![
                    OsString::from("image"),
                    OsString::from("--input"),
                    archive.clone().into_os_string(),
                    OsString::from("--offline-scan"),
                    OsString::from("--exit-code"),
                    OsString::from("1"),
                    OsString::from("--severity"),
                    OsString::from("CRITICAL,HIGH"),
                    OsString::from("--format"),
                    OsString::from("json"),
                    OsString::from("--output"),
                    scan.as_os_str().to_os_string(),
                ],
            )
            .await;
        fs::remove_file(&archive).map_err(OciWorkerError::Filesystem)?;
        scan_result?;
        if !scan.is_file()
            || fs::symlink_metadata(&scan)
                .map_err(OciWorkerError::Filesystem)?
                .file_type()
                .is_symlink()
        {
            return Err(OciWorkerError::InvalidOutput);
        }
        Ok(scan)
    }

    async fn sbom_layout(
        &self,
        layout: &Path,
        tooling: &ForgeZotPublicationConfig,
    ) -> Result<PathBuf, OciWorkerError> {
        let sbom = layout.join("sbom.spdx.json");
        self.command_success(
            &tooling.syft_binary,
            vec![
                OsString::from("scan"),
                OsString::from(format!("oci-dir:{}", layout.display())),
                OsString::from("--config"),
                tooling.syft_config.as_os_str().to_owned(),
                OsString::from("--output"),
                OsString::from(format!("spdx-json={}", sbom.display())),
                OsString::from("--quiet"),
            ],
        )
        .await?;
        trusted_output_file(layout, &sbom)
    }

    /// Builds the evidence bundle used by the credentialed publisher.
    ///
    /// # Errors
    ///
    /// Returns an error when local publication or evidence validation fails.
    pub async fn publication_material(
        &self,
        request: &IsolatedOciBuild,
        tooling: &ForgeZotPublicationConfig,
    ) -> Result<PreparedPublicationMaterial, OciWorkerError> {
        let image_name = self.buildah_image_name(request);
        let layout = self.publish_layout(request, &image_name).await?;
        let expected_manifest = layout_index_descriptor(&layout)?;
        let scan = self.scan_layout(request, &image_name, &layout).await?;
        let sbom = self.sbom_layout(&layout, tooling).await?;
        let provenance = layout.join("provenance.json");
        let provenance_document = super::output::ProvenanceDocument {
            version: 1,
            project_id: request.project_id,
            image_id: request.image_id.as_uuid(),
            base_reference: request.base_reference.as_str(),
            manifest_digest: expected_manifest.digest().as_str(),
        };
        fs::write(
            &provenance,
            serde_json::to_vec(&provenance_document).map_err(OciWorkerError::Serialization)?,
        )
        .map_err(OciWorkerError::Filesystem)?;
        let provenance = trusted_output_file(&layout, &provenance)?;
        Ok(PreparedPublicationMaterial {
            expected_manifest,
            material: PublicationMaterial {
                layout: layout.clone(),
                evidence: PublicationEvidenceFiles {
                    sbom,
                    provenance,
                    scan,
                    signature: None,
                },
            },
            layout,
        })
    }
}
