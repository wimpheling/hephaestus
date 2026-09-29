use registry_domain::{
    ImmutableManifestReference, OciDescriptor, OciMediaType, Sha256Digest, SupplyChainPolicy,
};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
};
use tempfile::TempDir;

use super::{
    config::PublicationEvidenceFiles,
    constants::{
        OCI_INDEX_MEDIA_TYPE, OCI_MANIFEST_MEDIA_TYPE, OCI_REFERENCE_NAME_ANNOTATION,
        PROVENANCE_ARTIFACT_TYPE, SBOM_ARTIFACT_TYPE, SCAN_ARTIFACT_TYPE, SIGNATURE_ARTIFACT_TYPE,
    },
    errors::PublisherError,
    paths::{local_reference_tag, trusted_file},
};

#[derive(Debug)]
pub struct ValidatedMaterial {
    pub layout: PathBuf,
    pub source_tag: String,
    pub subject: OciDescriptor,
    pub evidence: Vec<ValidatedEvidenceFile>,
}

#[derive(Debug)]
pub struct ValidatedEvidenceFile {
    pub path: PathBuf,
    pub artifact_type: &'static str,
}

/// A transient OCI layout containing one evidence artifact and its subject.
///
/// Skopeo publishes this standard layout with the same scoped bearer flow as
/// the image graph; the publisher never delegates bearer-token handling to an
/// artifact CLI.
pub struct EvidenceLayout {
    root: TempDir,
    tag: String,
}

impl EvidenceLayout {
    pub fn create(
        parent: &Path,
        _reference: &ImmutableManifestReference,
        subject: &OciDescriptor,
        evidence: &ValidatedEvidenceFile,
    ) -> Result<Self, PublisherError> {
        let root = TempDir::new_in(parent).map_err(PublisherError::Filesystem)?;
        let blobs = root.path().join("blobs/sha256");
        fs::create_dir_all(&blobs).map_err(PublisherError::Filesystem)?;
        fs::write(
            root.path().join("oci-layout"),
            r#"{"imageLayoutVersion":"1.0.0"}"#,
        )
        .map_err(PublisherError::Filesystem)?;
        let config = write_layout_blob(&blobs, b"{}")?;
        let evidence_bytes = fs::read(&evidence.path).map_err(PublisherError::Filesystem)?;
        let layer = write_layout_blob(&blobs, &evidence_bytes)?;
        let manifest = serde_json::json!({
            "schemaVersion": 2,
            "mediaType": OCI_MANIFEST_MEDIA_TYPE,
            "artifactType": evidence.artifact_type,
            "config": descriptor_json(&config, "application/vnd.oci.empty.v1+json"),
            "layers": [descriptor_json(&layer, evidence.artifact_type)],
            "subject": descriptor_json(subject, subject.media_type().as_str()),
        });
        let manifest_bytes =
            serde_json::to_vec(&manifest).map_err(|_| PublisherError::MalformedLocalLayout)?;
        let descriptor = write_layout_blob(&blobs, &manifest_bytes)?;
        let tag = local_reference_tag(descriptor.digest());
        let index = serde_json::json!({
            "schemaVersion": 2,
            "mediaType": OCI_INDEX_MEDIA_TYPE,
            "manifests": [{
                "mediaType": OCI_MANIFEST_MEDIA_TYPE,
                "digest": descriptor.digest().as_str(),
                "size": descriptor.size(),
                "annotations": { OCI_REFERENCE_NAME_ANNOTATION: tag },
            }],
        });
        fs::write(
            root.path().join("index.json"),
            serde_json::to_vec(&index).map_err(|_| PublisherError::MalformedLocalIndex)?,
        )
        .map_err(PublisherError::Filesystem)?;
        Ok(Self { root, tag })
    }

    pub fn path(&self) -> &Path {
        self.root.path()
    }
    pub fn tag(&self) -> &str {
        &self.tag
    }
}

fn write_layout_blob(blobs: &Path, bytes: &[u8]) -> Result<OciDescriptor, PublisherError> {
    let digest = Sha256Digest::parse(format!("sha256:{:x}", Sha256::digest(bytes)))
        .map_err(PublisherError::Domain)?;
    fs::write(blobs.join(&digest.as_str()["sha256:".len()..]), bytes)
        .map_err(PublisherError::Filesystem)?;
    OciDescriptor::new(
        digest,
        u64::try_from(bytes.len()).map_err(|_| PublisherError::MalformedLocalLayout)?,
        OciMediaType::parse(OCI_MANIFEST_MEDIA_TYPE.to_owned()).map_err(PublisherError::Domain)?,
    )
    .map_err(PublisherError::Domain)
}

fn descriptor_json(descriptor: &OciDescriptor, media_type: &str) -> serde_json::Value {
    serde_json::json!({
        "mediaType": media_type,
        "digest": descriptor.digest().as_str(),
        "size": descriptor.size(),
    })
}

pub fn validated_evidence(
    root: &Path,
    evidence: &PublicationEvidenceFiles,
    policy: SupplyChainPolicy,
) -> Result<Vec<ValidatedEvidenceFile>, PublisherError> {
    let mut files = vec![
        ValidatedEvidenceFile {
            path: trusted_file(root, &evidence.sbom)?,
            artifact_type: SBOM_ARTIFACT_TYPE,
        },
        ValidatedEvidenceFile {
            path: trusted_file(root, &evidence.provenance)?,
            artifact_type: PROVENANCE_ARTIFACT_TYPE,
        },
        ValidatedEvidenceFile {
            path: trusted_file(root, &evidence.scan)?,
            artifact_type: SCAN_ARTIFACT_TYPE,
        },
    ];
    match (&evidence.signature, policy.signature_required()) {
        (Some(path), _) => files.push(ValidatedEvidenceFile {
            path: trusted_file(root, path)?,
            artifact_type: SIGNATURE_ARTIFACT_TYPE,
        }),
        (None, true) => return Err(PublisherError::MissingSignature),
        (None, false) => {}
    }
    Ok(files)
}
