//! Verified output export and publication evidence helpers.

use super::{
    config::LocalOciRuntime, constants::VerifiedVmOciOutput, filesystem::copy_verified_rootfs,
    publication::PreparedPublicationMaterial,
};
use async_trait::async_trait;
use builder_catalog_domain::{OciDigest, OciImageReference};
use oci_builder_worker::{
    IsolatedOciBuild, OciImageProductionOutput, OciOutputPublisher, OciRootfsExporter,
    OciWorkerError,
};
use registry_domain::{OciDescriptor, OciMediaType};
use registry_publisher::{PublicationEvidenceFiles, PublicationMaterial};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    ffi::OsString,
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

/// The local runtime cannot publish repository-image output by itself.
///
/// This compatibility implementation deliberately fails closed until the
/// daemon composes [`ForgeZotOciPublisher`] with durable registry and token
/// ports. It replaces the former fabricated registry-like output path.
#[async_trait]
impl OciOutputPublisher for LocalOciRuntime {
    async fn publish(
        &self,
        _request: &IsolatedOciBuild,
    ) -> Result<OciImageProductionOutput, OciWorkerError> {
        Err(OciWorkerError::RegistryPublication)
    }
}

#[async_trait]
impl OciRootfsExporter for LocalOciRuntime {
    async fn export_rootfs(
        &self,
        image_reference: &OciImageReference,
        destination: &Path,
    ) -> Result<(), OciWorkerError> {
        if let Some(rootfs) = self.verified_rootfs_for(image_reference)? {
            return copy_verified_rootfs(&rootfs, destination);
        }
        let layout = self
            .config
            .image_layouts
            .get(image_reference.as_str())
            .ok_or(OciWorkerError::ImageNotCached)?;
        let umoci = self
            .config
            .umoci_binary
            .as_deref()
            .ok_or(OciWorkerError::ImageNotCached)?;
        let bundle = destination.join(".umoci-bundle");
        self.command_success(
            umoci,
            vec![
                OsString::from("unpack"),
                OsString::from("--rootless"),
                OsString::from("--image"),
                OsString::from(format!("{}:latest", layout.display())),
                bundle.as_os_str().to_os_string(),
            ],
        )
        .await?;
        let rootfs = bundle.join("rootfs");
        let entries = fs::read_dir(&rootfs).map_err(OciWorkerError::Filesystem)?;
        for entry in entries {
            let entry = entry.map_err(OciWorkerError::Filesystem)?;
            fs::rename(entry.path(), destination.join(entry.file_name()))
                .map_err(OciWorkerError::Filesystem)?;
        }
        fs::remove_dir_all(bundle).map_err(OciWorkerError::Filesystem)
    }
}

#[derive(Serialize)]
pub struct ProvenanceDocument<'a> {
    pub version: u32,
    pub project_id: uuid::Uuid,
    pub image_id: uuid::Uuid,
    pub base_reference: &'a str,
    pub manifest_digest: &'a str,
}

/// Binds independently verified VM output to the exact publication request.
///
/// # Errors
///
/// Returns an error when the sealed layout descriptor does not match the
/// verifier's observed digest or evidence cannot be safely retained.
pub fn verified_vm_publication_material(
    request: &IsolatedOciBuild,
    output: &VerifiedVmOciOutput,
) -> Result<PreparedPublicationMaterial, OciWorkerError> {
    let expected_manifest = layout_index_descriptor(&output.layout)?;
    if expected_manifest.digest().as_str() != output.manifest_digest.as_str() {
        return Err(OciWorkerError::InvalidOutput);
    }
    let evidence_root = output.sbom.parent().ok_or(OciWorkerError::InvalidOutput)?;
    let scan = trusted_output_file(evidence_root, &output.scan)?;
    let sbom = trusted_output_file(evidence_root, &output.sbom)?;
    let provenance = evidence_root.join("provenance.json");
    if provenance.exists() {
        return Err(OciWorkerError::InvalidOutput);
    }
    let document = ProvenanceDocument {
        version: 1,
        project_id: request.project_id,
        image_id: request.image_id.as_uuid(),
        base_reference: request.base_reference.as_str(),
        manifest_digest: output.manifest_digest.as_str(),
    };
    fs::write(
        &provenance,
        serde_json::to_vec(&document).map_err(OciWorkerError::Serialization)?,
    )
    .map_err(OciWorkerError::Filesystem)?;
    let provenance = trusted_output_file(evidence_root, &provenance)?;
    Ok(PreparedPublicationMaterial {
        expected_manifest,
        material: PublicationMaterial {
            layout: output.layout.clone(),
            evidence: PublicationEvidenceFiles {
                sbom,
                provenance,
                scan,
                signature: None,
            },
        },
        layout: output.layout.clone(),
    })
}

pub fn validate_executable(path: &Path) -> Result<(), OciWorkerError> {
    let metadata = fs::symlink_metadata(path).map_err(OciWorkerError::Filesystem)?;
    if !path.is_absolute()
        || metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.permissions().mode() & 0o111 == 0
    {
        return Err(OciWorkerError::InvalidConfiguration);
    }
    Ok(())
}

pub fn canonical_regular_file(path: &Path) -> Result<PathBuf, OciWorkerError> {
    if !path.is_absolute() {
        return Err(OciWorkerError::InvalidConfiguration);
    }
    let metadata = fs::symlink_metadata(path).map_err(OciWorkerError::Filesystem)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(OciWorkerError::InvalidConfiguration);
    }
    fs::canonicalize(path).map_err(OciWorkerError::Filesystem)
}

pub fn initialize_directory(path: &Path) -> Result<PathBuf, OciWorkerError> {
    if !path.is_absolute() {
        return Err(OciWorkerError::InvalidConfiguration);
    }
    fs::create_dir_all(path).map_err(OciWorkerError::Filesystem)?;
    canonical_directory(path)
}

pub fn canonical_directory(path: &Path) -> Result<PathBuf, OciWorkerError> {
    let path = fs::canonicalize(path).map_err(OciWorkerError::Filesystem)?;
    let metadata = fs::symlink_metadata(&path).map_err(OciWorkerError::Filesystem)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(OciWorkerError::InvalidConfiguration);
    }
    Ok(path)
}

pub fn prepare_job_checkout(root: &Path, job_id: uuid::Uuid) -> Result<PathBuf, OciWorkerError> {
    let checkout = root.join(job_id.to_string());
    let metadata = match fs::symlink_metadata(&checkout) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(checkout),
        Err(error) => return Err(OciWorkerError::Filesystem(error)),
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(OciWorkerError::UnsafeSourcePath);
    }
    let canonical = fs::canonicalize(&checkout).map_err(OciWorkerError::Filesystem)?;
    if canonical.parent() != Some(root) {
        return Err(OciWorkerError::UnsafeSourcePath);
    }
    fs::remove_dir_all(canonical).map_err(OciWorkerError::Filesystem)?;
    Ok(checkout)
}

pub fn normalize_layout_to_single_index(layout: &Path) -> Result<PathBuf, OciWorkerError> {
    let index_path = layout.join("index.json");
    let source: serde_json::Value =
        serde_json::from_slice(&fs::read(&index_path).map_err(OciWorkerError::Filesystem)?)
            .map_err(OciWorkerError::Serialization)?;
    let manifests = source
        .get("manifests")
        .and_then(serde_json::Value::as_array)
        .filter(|manifests| manifests.len() == 1)
        .ok_or(OciWorkerError::InvalidOutput)?;
    let mut manifest = manifests[0].clone();
    let digest = manifest
        .get("digest")
        .and_then(serde_json::Value::as_str)
        .ok_or(OciWorkerError::InvalidOutput)?;
    let digest = OciDigest::parse(digest.to_owned()).map_err(|_| OciWorkerError::InvalidOutput)?;
    let size = manifest
        .get("size")
        .and_then(serde_json::Value::as_u64)
        .filter(|size| *size > 0)
        .ok_or(OciWorkerError::InvalidOutput)?;
    manifest
        .get("mediaType")
        .and_then(serde_json::Value::as_str)
        .filter(|media_type| *media_type == OciMediaType::IMAGE_MANIFEST)
        .ok_or(OciWorkerError::InvalidOutput)?;
    let blob = layout.join("blobs").join("sha256").join(
        digest
            .as_str()
            .strip_prefix("sha256:")
            .ok_or(OciWorkerError::InvalidOutput)?,
    );
    let blob_bytes = fs::read(&blob).map_err(OciWorkerError::Filesystem)?;
    if u64::try_from(blob_bytes.len()).map_err(|_| OciWorkerError::InvalidOutput)? != size
        || format!("sha256:{:x}", Sha256::digest(&blob_bytes)) != digest.as_str()
    {
        return Err(OciWorkerError::InvalidOutput);
    }
    let object = manifest
        .as_object_mut()
        .ok_or(OciWorkerError::InvalidOutput)?;
    object.insert(
        String::from("platform"),
        serde_json::json!({ "os": "linux", "architecture": "amd64" }),
    );
    let index_bytes = serde_json::to_vec(&serde_json::json!({
        "schemaVersion": 2,
        "mediaType": OciMediaType::IMAGE_INDEX,
        "manifests": [manifest],
    }))
    .map_err(OciWorkerError::Serialization)?;
    let index_digest = format!("sha256:{:x}", Sha256::digest(&index_bytes));
    let index_blob = layout.join("blobs").join("sha256").join(
        index_digest
            .strip_prefix("sha256:")
            .ok_or(OciWorkerError::InvalidOutput)?,
    );
    fs::write(&index_blob, &index_bytes).map_err(OciWorkerError::Filesystem)?;
    let reference_tag = format!("heph-{}", index_digest.replace(':', "-"));
    fs::write(
        &index_path,
        serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 2,
            "manifests": [{
                "mediaType": OciMediaType::IMAGE_INDEX,
                "digest": index_digest,
                "size": index_bytes.len(),
                "annotations": { "org.opencontainers.image.ref.name": reference_tag },
            }],
        }))
        .map_err(OciWorkerError::Serialization)?,
    )
    .map_err(OciWorkerError::Filesystem)?;
    Ok(layout.to_path_buf())
}

pub fn layout_index_descriptor(layout: &Path) -> Result<OciDescriptor, OciWorkerError> {
    let index: serde_json::Value = serde_json::from_slice(
        &fs::read(layout.join("index.json")).map_err(OciWorkerError::Filesystem)?,
    )
    .map_err(OciWorkerError::Serialization)?;
    let descriptor = index
        .get("manifests")
        .and_then(serde_json::Value::as_array)
        .and_then(|manifests| manifests.first())
        .ok_or(OciWorkerError::InvalidOutput)?;
    let digest = descriptor
        .get("digest")
        .and_then(serde_json::Value::as_str)
        .ok_or(OciWorkerError::InvalidOutput)?;
    let size = descriptor
        .get("size")
        .and_then(serde_json::Value::as_u64)
        .ok_or(OciWorkerError::InvalidOutput)?;
    OciDescriptor::new(
        registry_domain::Sha256Digest::parse(digest.to_owned())
            .map_err(|_| OciWorkerError::InvalidOutput)?,
        size,
        OciMediaType::parse(OciMediaType::IMAGE_INDEX)
            .map_err(|_| OciWorkerError::InvalidOutput)?,
    )
    .map_err(|_| OciWorkerError::InvalidOutput)
}

pub fn trusted_output_file(root: &Path, file: &Path) -> Result<PathBuf, OciWorkerError> {
    let file = fs::canonicalize(file).map_err(OciWorkerError::Filesystem)?;
    let metadata = fs::symlink_metadata(&file).map_err(OciWorkerError::Filesystem)?;
    (file.starts_with(root) && metadata.is_file() && !metadata.file_type().is_symlink())
        .then_some(file)
        .ok_or(OciWorkerError::InvalidOutput)
}
