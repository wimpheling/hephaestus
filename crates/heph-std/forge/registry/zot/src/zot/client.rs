//! Exact-digest inspection and reconciliation boundary.

use async_trait::async_trait;
use registry_domain::{
    ImmutableManifestReference, OciMediaType, Sha256Digest, SupplyChainEvidence,
    SupplyChainReferrer,
};
use registry_reconciler::{ReconciliationPortError, ZotInspection, ZotRegistry};
use reqwest::Client;
use std::sync::Arc;

use super::model::{
    OCI_MANIFEST_MEDIA_TYPE, REQUEST_TIMEOUT, RegistryPullTokenProvider, ZotClientConfig,
    ZotClientError, ZotHttpRegistry,
};
use super::protocol::{RemoteDescriptor, RemoteIndex, artifact_type, referrer_kind};

impl<T> ZotHttpRegistry<T>
where
    T: RegistryPullTokenProvider,
{
    /// Builds a redirect-free bounded HTTP client.
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying HTTP client cannot be configured.
    pub fn new(config: ZotClientConfig, tokens: Arc<T>) -> Result<Self, ZotClientError> {
        let client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|_| ZotClientError::InvalidConfiguration)?;
        Ok(Self {
            config,
            client,
            tokens,
        })
    }

    pub(in crate::zot) async fn inspect_exact(
        &self,
        reference: &ImmutableManifestReference,
    ) -> Result<ZotInspection, ZotClientError> {
        if reference.authority() != &self.config.authority {
            return Err(ZotClientError::AuthorityMismatch);
        }
        let issued = self.tokens.issue_pull_token(reference.namespace()).await?;
        let token = issued.token().as_str();
        let Some(subject) = self
            .fetch_manifest(reference.namespace(), reference.digest(), token, true)
            .await?
        else {
            return Ok(ZotInspection::Missing);
        };
        if !subject.descriptor.media_type().is_image_index() {
            return Err(ZotClientError::InvalidGraph);
        }
        let platforms = subject
            .document
            .manifests
            .iter()
            .map(RemoteDescriptor::to_platform)
            .collect::<Result<Vec<_>, _>>()?;
        if platforms.is_empty() {
            return Err(ZotClientError::InvalidGraph);
        }
        let referrer_index = self
            .get_json(
                reference.namespace(),
                &format!("referrers/{}", reference.digest()),
                token,
            )
            .await?;
        let discovered: RemoteIndex =
            serde_json::from_slice(&referrer_index).map_err(|_| ZotClientError::InvalidGraph)?;
        let mut evidence = Vec::new();
        for descriptor in &discovered.manifests {
            let Some(kind) = referrer_kind(descriptor.artifact_type.as_deref()) else {
                continue;
            };
            let digest = Sha256Digest::parse(descriptor.digest.clone())?;
            let Some(remote) = self
                .fetch_manifest(reference.namespace(), &digest, token, false)
                .await?
            else {
                return Err(ZotClientError::InvalidGraph);
            };
            let expected_artifact_type = artifact_type(kind);
            if remote.document.media_type.as_deref() != Some(OCI_MANIFEST_MEDIA_TYPE)
                || remote.document.artifact_type.as_deref() != Some(expected_artifact_type)
                || remote.document.layers.is_empty()
                || remote
                    .document
                    .subject
                    .as_ref()
                    .map(|value| value.digest.as_str())
                    != Some(reference.digest().as_str())
                || remote.descriptor != descriptor.to_domain()?
            {
                return Err(ZotClientError::InvalidGraph);
            }
            evidence.push(SupplyChainReferrer::new(
                kind,
                reference.digest().clone(),
                remote.descriptor,
                OciMediaType::parse(expected_artifact_type)?,
            ));
        }
        Ok(ZotInspection::Present {
            manifest: subject.descriptor,
            platforms,
            evidence: SupplyChainEvidence::new(reference.digest().clone(), evidence)?,
        })
    }
}

#[async_trait]
impl<T> ZotRegistry for ZotHttpRegistry<T>
where
    T: RegistryPullTokenProvider,
{
    async fn inspect(
        &self,
        reference: &ImmutableManifestReference,
    ) -> Result<ZotInspection, ReconciliationPortError> {
        match self.inspect_exact(reference).await {
            Ok(inspection) => Ok(inspection),
            Err(
                ZotClientError::InvalidGraph
                | ZotClientError::ResponseTooLarge
                | ZotClientError::Domain(_),
            ) => Ok(ZotInspection::Invalid),
            Err(
                ZotClientError::InvalidConfiguration
                | ZotClientError::AuthorityMismatch
                | ZotClientError::Unavailable
                | ZotClientError::Unauthorized,
            ) => Err(ReconciliationPortError),
        }
    }
}
