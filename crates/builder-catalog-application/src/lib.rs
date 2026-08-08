//! Application ports for immutable OCI images.

use async_trait::async_trait;
use builder_catalog_domain::{
    ImageCatalogValueError, ImageKey, ImageSelectionError, OciImage, OciImageId,
    OciImagePublication, OciImageReference, ResolvedImage,
};
use identity_domain::AuthenticatedIdentity;
use std::sync::Arc;

/// Persistence failure for the OCI image catalog.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ImageCatalogError {
    /// No image matches the requested immutable identity.
    #[error("OCI image was not found")]
    NotFound,
    /// Storage or transport failed.
    #[error("OCI image catalog storage failed: {0}")]
    Storage(#[source] Box<dyn std::error::Error + Send + Sync>),
    /// Persisted metadata violates the domain contract.
    #[error("OCI image catalog contains invalid metadata: {0}")]
    InvalidData(#[source] ImageCatalogValueError),
}

/// Persistence boundary for immutable OCI images.
#[async_trait]
pub trait ImageCatalog: Send + Sync + 'static {
    /// Lists platform catalog entries in stable key order.
    async fn list_images(&self) -> Result<Vec<OciImage>, ImageCatalogError>;

    /// Loads one entry by its stable identity.
    async fn get_image(&self, id: OciImageId) -> Result<OciImage, ImageCatalogError>;

    /// Loads one entry by its immutable OCI reference.
    async fn find_image_by_reference(
        &self,
        reference: &OciImageReference,
    ) -> Result<OciImage, ImageCatalogError>;

    /// Loads one entry by its human-selected stable key.
    async fn find_image_by_key(&self, key: &ImageKey) -> Result<OciImage, ImageCatalogError> {
        self.list_images()
            .await?
            .into_iter()
            .find(|image| image.key == *key)
            .ok_or(ImageCatalogError::NotFound)
    }
}

/// Read boundary for safe OCI registry-publication projections.
#[async_trait]
pub trait RegistryPublicationCatalog: Send + Sync + 'static {
    /// Lists platform images with their registry state and evidence.
    async fn list_image_publications(
        &self,
        identity: &AuthenticatedIdentity,
    ) -> Result<Vec<OciImagePublication>, ImageCatalogError>;

    /// Loads one platform image with its registry state and evidence.
    async fn get_image_publication(
        &self,
        identity: &AuthenticatedIdentity,
        id: OciImageId,
    ) -> Result<OciImagePublication, ImageCatalogError>;
}

/// Resolves image keys to immutable OCI provenance for any execution context.
pub struct ImageCatalogApplication<C> {
    catalog: Arc<C>,
}

impl<C> ImageCatalogApplication<C>
where
    C: ImageCatalog,
{
    /// Creates the service over an OCI image catalog.
    #[must_use]
    pub const fn new(catalog: Arc<C>) -> Self {
        Self { catalog }
    }

    /// Lists validated image metadata.
    ///
    /// # Errors
    ///
    /// Returns a storage or invalid-data error.
    pub async fn list_images(&self) -> Result<Vec<OciImage>, ImageCatalogError> {
        self.catalog
            .list_images()
            .await?
            .into_iter()
            .map(validate_image)
            .collect()
    }

    /// Loads validated image metadata.
    ///
    /// # Errors
    ///
    /// Returns a storage or invalid-data error.
    pub async fn get_image(&self, id: OciImageId) -> Result<OciImage, ImageCatalogError> {
        validate_image(self.catalog.get_image(id).await?)
    }

    /// Resolves a selected catalog key to an immutable OCI reference.
    ///
    /// Execution-specific resource, network, secret, and mount policy is
    /// deliberately not considered here: it belongs to the calling context.
    ///
    /// # Errors
    ///
    /// Returns a catalog failure or an unavailable/retired selection error.
    pub async fn resolve_key(
        &self,
        key: &ImageKey,
    ) -> Result<ResolvedImage, ImageCatalogApplicationError> {
        let image = self.catalog.find_image_by_key(key).await?;
        validate_image(image)?.resolve().map_err(Into::into)
    }

    /// Resolves an immutable reference already held in durable provenance.
    ///
    /// # Errors
    ///
    /// Returns a catalog failure or an unavailable/retired selection error.
    pub async fn resolve_reference(
        &self,
        reference: &OciImageReference,
    ) -> Result<ResolvedImage, ImageCatalogApplicationError> {
        let image = self.catalog.find_image_by_reference(reference).await?;
        validate_image(image)?.resolve().map_err(Into::into)
    }
}

impl<C> ImageCatalogApplication<C>
where
    C: ImageCatalog + RegistryPublicationCatalog,
{
    /// Lists validated catalog and registry-publication metadata.
    ///
    /// # Errors
    ///
    /// Returns a storage or invalid-data error.
    pub async fn list_image_publications(
        &self,
        identity: &AuthenticatedIdentity,
    ) -> Result<Vec<OciImagePublication>, ImageCatalogError> {
        self.catalog
            .list_image_publications(identity)
            .await?
            .into_iter()
            .map(|publication| {
                publication
                    .validate()
                    .map(|()| publication)
                    .map_err(ImageCatalogError::InvalidData)
            })
            .collect()
    }

    /// Loads validated catalog and registry-publication metadata.
    ///
    /// # Errors
    ///
    /// Returns a storage or invalid-data error.
    pub async fn get_image_publication(
        &self,
        identity: &AuthenticatedIdentity,
        id: OciImageId,
    ) -> Result<OciImagePublication, ImageCatalogError> {
        let publication = self.catalog.get_image_publication(identity, id).await?;
        publication
            .validate()
            .map(|()| publication)
            .map_err(ImageCatalogError::InvalidData)
    }
}

fn validate_image(image: OciImage) -> Result<OciImage, ImageCatalogError> {
    image
        .validate()
        .map(|()| image)
        .map_err(ImageCatalogError::InvalidData)
}

/// Application-level OCI image failure.
#[derive(Debug, thiserror::Error)]
pub enum ImageCatalogApplicationError {
    /// Image catalog lookup or validation failed.
    #[error(transparent)]
    Catalog(#[from] ImageCatalogError),
    /// The selected image cannot be used for new work.
    #[error(transparent)]
    Selection(#[from] ImageSelectionError),
}
