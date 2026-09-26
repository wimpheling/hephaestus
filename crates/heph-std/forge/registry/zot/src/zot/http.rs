//! Bounded HTTP fetch helpers for Zot responses.

use registry_domain::{OciDescriptor, OciMediaType, RegistryNamespace, Sha256Digest};
use reqwest::{StatusCode, Url, header};
use sha2::{Digest, Sha256};

use super::model::{
    MAX_MANIFEST_BYTES, OCI_INDEX_MEDIA_TYPE, OCI_MANIFEST_MEDIA_TYPE, ZotClientError,
    ZotHttpRegistry,
};
use super::protocol::{FetchedManifest, RemoteManifest};

impl<T> ZotHttpRegistry<T>
where
    T: super::model::RegistryPullTokenProvider,
{
    pub(in crate::zot) async fn fetch_manifest(
        &self,
        namespace: &RegistryNamespace,
        digest: &Sha256Digest,
        token: &str,
        allow_missing: bool,
    ) -> Result<Option<FetchedManifest>, ZotClientError> {
        let url = self.url(namespace, &format!("manifests/{digest}"))?;
        let response = self
            .client
            .get(url)
            .bearer_auth(token)
            .header(
                header::ACCEPT,
                format!("{OCI_INDEX_MEDIA_TYPE}, {OCI_MANIFEST_MEDIA_TYPE}"),
            )
            .send()
            .await
            .map_err(|_| ZotClientError::Unavailable)?;
        if allow_missing && response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if response.status() != StatusCode::OK {
            return Err(status_error(response.status()));
        }
        let advertised_digest = response
            .headers()
            .get("docker-content-digest")
            .and_then(|value| value.to_str().ok())
            .ok_or(ZotClientError::InvalidGraph)?;
        if advertised_digest != digest.as_str() {
            return Err(ZotClientError::InvalidGraph);
        }
        let bytes = bounded_bytes(response).await?;
        let actual = Sha256Digest::parse(format!("sha256:{:x}", Sha256::digest(&bytes)))?;
        if &actual != digest {
            return Err(ZotClientError::InvalidGraph);
        }
        let document: RemoteManifest =
            serde_json::from_slice(&bytes).map_err(|_| ZotClientError::InvalidGraph)?;
        let media_type = OciMediaType::parse(
            document
                .media_type
                .clone()
                .ok_or(ZotClientError::InvalidGraph)?,
        )?;
        let descriptor = OciDescriptor::new(
            actual,
            u64::try_from(bytes.len()).map_err(|_| ZotClientError::ResponseTooLarge)?,
            media_type,
        )?;
        Ok(Some(FetchedManifest {
            descriptor,
            document,
        }))
    }

    pub(in crate::zot) async fn get_json(
        &self,
        namespace: &RegistryNamespace,
        suffix: &str,
        token: &str,
    ) -> Result<Vec<u8>, ZotClientError> {
        let response = self
            .client
            .get(self.url(namespace, suffix)?)
            .bearer_auth(token)
            .header(header::ACCEPT, OCI_INDEX_MEDIA_TYPE)
            .send()
            .await
            .map_err(|_| ZotClientError::Unavailable)?;
        if response.status() != StatusCode::OK {
            return Err(status_error(response.status()));
        }
        bounded_bytes(response).await
    }

    pub(in crate::zot) fn url(
        &self,
        namespace: &RegistryNamespace,
        suffix: &str,
    ) -> Result<Url, ZotClientError> {
        self.config
            .private_origin
            .join(&format!("v2/{}/{suffix}", namespace.as_str()))
            .map_err(|_| ZotClientError::InvalidConfiguration)
    }
}

async fn bounded_bytes(response: reqwest::Response) -> Result<Vec<u8>, ZotClientError> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_MANIFEST_BYTES as u64)
    {
        return Err(ZotClientError::ResponseTooLarge);
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|_| ZotClientError::Unavailable)?;
    if bytes.is_empty() || bytes.len() > MAX_MANIFEST_BYTES {
        return Err(ZotClientError::ResponseTooLarge);
    }
    Ok(bytes.to_vec())
}

fn status_error(status: StatusCode) -> ZotClientError {
    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        ZotClientError::Unauthorized
    } else {
        ZotClientError::Unavailable
    }
}
