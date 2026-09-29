use registry_domain::{
    OciDescriptor, OciMediaType, PlatformDescriptor, Sha256Digest, SupplyChainPolicy,
    SupplyChainReferrerKind,
};
use reqwest::blocking::Response as HttpResponse;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, io::Read};

use super::{
    constants::{
        MAX_REGISTRY_DOCUMENT_BYTES, OCI_INDEX_MEDIA_TYPE, PROVENANCE_ARTIFACT_TYPE,
        SBOM_ARTIFACT_TYPE, SCAN_ARTIFACT_TYPE, SIGNATURE_ARTIFACT_TYPE,
    },
    errors::{PublisherError, RegistryReadError},
};

pub const fn artifact_type(kind: SupplyChainReferrerKind) -> &'static str {
    match kind {
        SupplyChainReferrerKind::Sbom => SBOM_ARTIFACT_TYPE,
        SupplyChainReferrerKind::Provenance => PROVENANCE_ARTIFACT_TYPE,
        SupplyChainReferrerKind::Scan => SCAN_ARTIFACT_TYPE,
        SupplyChainReferrerKind::Signature => SIGNATURE_ARTIFACT_TYPE,
    }
}

pub fn required_kinds(
    policy: SupplyChainPolicy,
    signature_present: bool,
) -> Vec<SupplyChainReferrerKind> {
    let mut kinds = vec![
        SupplyChainReferrerKind::Sbom,
        SupplyChainReferrerKind::Provenance,
        SupplyChainReferrerKind::Scan,
    ];
    if policy.signature_required() || signature_present {
        kinds.push(SupplyChainReferrerKind::Signature);
    }
    kinds
}
pub fn bounded_response(response: HttpResponse) -> Result<Vec<u8>, RegistryReadError> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_REGISTRY_DOCUMENT_BYTES as u64)
    {
        return Err(RegistryReadError::Failed);
    }
    let mut bytes = Vec::new();
    let mut limited = response.take(u64::try_from(MAX_REGISTRY_DOCUMENT_BYTES + 1).expect("bound"));
    limited
        .read_to_end(&mut bytes)
        .map_err(|_| RegistryReadError::Failed)?;
    (!bytes.is_empty() && bytes.len() <= MAX_REGISTRY_DOCUMENT_BYTES)
        .then_some(bytes)
        .ok_or(RegistryReadError::Failed)
}

pub fn validate_manifest_bytes(
    bytes: &[u8],
    descriptor: &OciDescriptor,
) -> Result<(), PublisherError> {
    if u64::try_from(bytes.len()).map_err(|_| PublisherError::WrongRemoteManifestBytes)?
        != descriptor.size()
        || Sha256Digest::parse(format!("sha256:{:x}", Sha256::digest(bytes)))
            .map_err(PublisherError::Domain)?
            != *descriptor.digest()
    {
        return Err(PublisherError::WrongRemoteManifestBytes);
    }
    Ok(())
}

pub fn parse_platforms(
    manifest: &RemoteManifest,
    descriptor: &OciDescriptor,
) -> Result<Vec<PlatformDescriptor>, PublisherError> {
    if manifest.media_type.as_deref() != Some(OCI_INDEX_MEDIA_TYPE)
        || !descriptor.media_type().is_image_index()
        || manifest.manifests.is_empty()
    {
        return Err(PublisherError::MalformedRemoteIndex);
    }
    manifest
        .manifests
        .iter()
        .map(|entry| {
            let descriptor = entry.to_domain()?;
            let platform = entry
                .platform
                .as_ref()
                .ok_or(PublisherError::MalformedRemoteIndex)?;
            PlatformDescriptor::new(
                descriptor,
                platform.operating_system.clone(),
                platform.architecture.clone(),
                platform.variant.clone(),
            )
            .map_err(PublisherError::Domain)
        })
        .collect()
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteDescriptor {
    pub media_type: String,
    pub digest: String,
    pub size: u64,
    #[serde(default)]
    pub artifact_type: Option<String>,
    #[serde(default)]
    pub platform: Option<RemotePlatform>,
    #[serde(default)]
    pub annotations: BTreeMap<String, String>,
}

impl RemoteDescriptor {
    pub fn to_domain(&self) -> Result<OciDescriptor, PublisherError> {
        OciDescriptor::new(
            Sha256Digest::parse(self.digest.clone()).map_err(PublisherError::Domain)?,
            self.size,
            OciMediaType::parse(self.media_type.clone()).map_err(PublisherError::Domain)?,
        )
        .map_err(PublisherError::Domain)
    }
}

pub type RemoteReferrerDescriptor = RemoteDescriptor;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemotePlatform {
    #[serde(rename = "os")]
    pub operating_system: String,
    pub architecture: String,
    #[serde(default)]
    pub variant: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RemoteReferrers {
    pub manifests: Vec<RemoteReferrerDescriptor>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteManifest {
    #[serde(default)]
    pub media_type: Option<String>,
    #[serde(default)]
    pub manifests: Vec<RemoteDescriptor>,
    #[serde(default)]
    pub subject: Option<RemoteSubject>,
    #[serde(default)]
    pub artifact_type: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RemoteSubject {
    pub digest: String,
}
