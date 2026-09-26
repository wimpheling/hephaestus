//! OCI descriptor decoding and supply-chain referrer validation helpers.

use registry_domain::{
    OciDescriptor, OciMediaType, PlatformDescriptor, Sha256Digest, SupplyChainReferrerKind,
};
use serde::Deserialize;

use super::model::{
    PROVENANCE_ARTIFACT_TYPE, SBOM_ARTIFACT_TYPE, SCAN_ARTIFACT_TYPE, SIGNATURE_ARTIFACT_TYPE,
    ZotClientError,
};

pub(in crate::zot) const fn artifact_type(kind: SupplyChainReferrerKind) -> &'static str {
    match kind {
        SupplyChainReferrerKind::Sbom => SBOM_ARTIFACT_TYPE,
        SupplyChainReferrerKind::Provenance => PROVENANCE_ARTIFACT_TYPE,
        SupplyChainReferrerKind::Scan => SCAN_ARTIFACT_TYPE,
        SupplyChainReferrerKind::Signature => SIGNATURE_ARTIFACT_TYPE,
    }
}

pub(in crate::zot) fn referrer_kind(value: Option<&str>) -> Option<SupplyChainReferrerKind> {
    match value {
        Some(SBOM_ARTIFACT_TYPE) => Some(SupplyChainReferrerKind::Sbom),
        Some(PROVENANCE_ARTIFACT_TYPE) => Some(SupplyChainReferrerKind::Provenance),
        Some(SCAN_ARTIFACT_TYPE) => Some(SupplyChainReferrerKind::Scan),
        Some(SIGNATURE_ARTIFACT_TYPE) => Some(SupplyChainReferrerKind::Signature),
        _ => None,
    }
}

pub(in crate::zot) struct FetchedManifest {
    pub(in crate::zot) descriptor: OciDescriptor,
    pub(in crate::zot) document: RemoteManifest,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::zot) struct RemoteManifest {
    pub(in crate::zot) media_type: Option<String>,
    #[serde(default)]
    pub(in crate::zot) artifact_type: Option<String>,
    #[serde(default)]
    pub(in crate::zot) subject: Option<RemoteSubject>,
    #[serde(default)]
    pub(in crate::zot) manifests: Vec<RemoteDescriptor>,
    #[serde(default)]
    pub(in crate::zot) layers: Vec<RemoteDescriptor>,
}

#[derive(Deserialize)]
pub(in crate::zot) struct RemoteSubject {
    pub(in crate::zot) digest: String,
}

#[derive(Deserialize)]
pub(in crate::zot) struct RemoteIndex {
    pub(in crate::zot) manifests: Vec<RemoteDescriptor>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::zot) struct RemoteDescriptor {
    pub(in crate::zot) media_type: String,
    pub(in crate::zot) digest: String,
    pub(in crate::zot) size: u64,
    #[serde(default)]
    pub(in crate::zot) artifact_type: Option<String>,
    #[serde(default)]
    platform: Option<RemotePlatform>,
}

impl RemoteDescriptor {
    pub(in crate::zot) fn to_domain(&self) -> Result<OciDescriptor, ZotClientError> {
        OciDescriptor::new(
            Sha256Digest::parse(self.digest.clone())?,
            self.size,
            OciMediaType::parse(self.media_type.clone())?,
        )
        .map_err(ZotClientError::Domain)
    }

    pub(in crate::zot) fn to_platform(&self) -> Result<PlatformDescriptor, ZotClientError> {
        let platform = self.platform.as_ref().ok_or(ZotClientError::InvalidGraph)?;
        PlatformDescriptor::new(
            self.to_domain()?,
            platform.operating_system.clone(),
            platform.architecture.clone(),
            platform.variant.clone(),
        )
        .map_err(ZotClientError::Domain)
    }
}

#[derive(Deserialize)]
struct RemotePlatform {
    #[serde(rename = "os")]
    operating_system: String,
    architecture: String,
    #[serde(default)]
    variant: Option<String>,
}
