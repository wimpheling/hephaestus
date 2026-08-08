//! `PostgreSQL` adapter for immutable OCI images.

use async_trait::async_trait;
use builder_catalog_application::{ImageCatalog, ImageCatalogError, RegistryPublicationCatalog};
use builder_catalog_domain::{
    AvailabilityState, ImageCatalogValueError, ImageKey, ImageProvenance, OciImage, OciImageId,
    OciImagePublication, OciImageReference, RegistryAvailabilityState, RegistryEvidence,
    RegistryPublication, RegistryPublicationState, Toolchain,
};
use identity_domain::AuthenticatedIdentity;
use serde_json::Value;
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

/// PostgreSQL-backed OCI image catalog.
#[derive(Clone)]
pub struct PgOciImageCatalog {
    pool: PgPool,
}

impl PgOciImageCatalog {
    /// Creates an adapter over the application database pool.
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    async fn platform_publication(
        &self,
        identity: &AuthenticatedIdentity,
        key: &ImageKey,
        reference: &OciImageReference,
    ) -> Result<RegistryPublication, ImageCatalogError> {
        let mut transaction = authz_postgres::begin_actor_transaction(&self.pool, identity)
            .await
            .map_err(storage)?;
        let row = sqlx::query_as::<_, RegistryPublicationRow>(
            "SELECT publication.state,
                    publication.registry_authority || '/' || namespace.repository_path
                        || '@' || publication.expected_digest AS immutable_reference,
                    COALESCE(ARRAY(
                        SELECT platform.architecture
                        FROM registry_publication_platforms platform
                        WHERE platform.publication_id = publication.id
                        ORDER BY platform.architecture, platform.variant, platform.digest
                    ), ARRAY[]::text[]) AS architectures,
                    (SELECT publication.registry_authority || '/' || namespace.repository_path
                        || '@' || evidence.digest
                     FROM registry_publication_evidence evidence
                     WHERE evidence.publication_id = publication.id AND evidence.kind = 'sbom')
                        AS sbom_reference,
                    (SELECT publication.registry_authority || '/' || namespace.repository_path
                        || '@' || evidence.digest
                     FROM registry_publication_evidence evidence
                     WHERE evidence.publication_id = publication.id AND evidence.kind = 'provenance')
                        AS provenance_reference,
                    (SELECT publication.registry_authority || '/' || namespace.repository_path
                        || '@' || evidence.digest
                     FROM registry_publication_evidence evidence
                     WHERE evidence.publication_id = publication.id AND evidence.kind = 'scan')
                        AS scan_reference,
                    (SELECT publication.registry_authority || '/' || namespace.repository_path
                        || '@' || evidence.digest
                     FROM registry_publication_evidence evidence
                     WHERE evidence.publication_id = publication.id AND evidence.kind = 'signature')
                        AS signature_reference,
                    publication.signature_required
             FROM registry_publications publication
             JOIN registry_namespaces namespace ON namespace.id = publication.namespace_id
             WHERE publication.owner_kind = 'platform_image'
               AND publication.platform_image_key = $1
               AND publication.registry_authority || '/' || namespace.repository_path
                   || '@' || publication.expected_digest = $2
             ORDER BY publication.created_at DESC, publication.id DESC
             LIMIT 1",
        )
        .bind(key.as_str())
        .bind(reference.as_str())
        .fetch_optional(&mut *transaction)
        .await
        .map_err(storage)?;
        transaction.commit().await.map_err(storage)?;
        row.map(RegistryPublicationRow::try_into_domain)
            .transpose()
            .map_err(invalid_data)?
            .ok_or(ImageCatalogError::NotFound)
    }
}

#[async_trait]
impl ImageCatalog for PgOciImageCatalog {
    async fn list_images(&self) -> Result<Vec<OciImage>, ImageCatalogError> {
        let rows = sqlx::query_as::<_, OciImageRow>(
            "SELECT id, key, display_name, image_reference, toolchains, architectures,
                    availability_state, provenance, signature_reference, sbom_reference,
                    platform_policy_version
             FROM oci_images ORDER BY key, id",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(storage)?;
        rows.into_iter().map(OciImageRow::try_into_domain).collect()
    }

    async fn get_image(&self, id: OciImageId) -> Result<OciImage, ImageCatalogError> {
        let row = sqlx::query_as::<_, OciImageRow>(
            "SELECT id, key, display_name, image_reference, toolchains, architectures,
                    availability_state, provenance, signature_reference, sbom_reference,
                    platform_policy_version
             FROM oci_images WHERE id = $1",
        )
        .bind(id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(storage)?
        .ok_or(ImageCatalogError::NotFound)?;
        row.try_into_domain()
    }

    async fn find_image_by_reference(
        &self,
        reference: &OciImageReference,
    ) -> Result<OciImage, ImageCatalogError> {
        let row = sqlx::query_as::<_, OciImageRow>(
            "SELECT id, key, display_name, image_reference, toolchains, architectures,
                    availability_state, provenance, signature_reference, sbom_reference,
                    platform_policy_version
             FROM oci_images WHERE image_reference = $1",
        )
        .bind(reference.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(storage)?
        .ok_or(ImageCatalogError::NotFound)?;
        row.try_into_domain()
    }
}

#[async_trait]
impl RegistryPublicationCatalog for PgOciImageCatalog {
    async fn list_image_publications(
        &self,
        identity: &AuthenticatedIdentity,
    ) -> Result<Vec<OciImagePublication>, ImageCatalogError> {
        let images = self.list_images().await?;
        let mut publications = Vec::with_capacity(images.len());
        for image in images {
            let registry_publication = self
                .platform_publication(identity, &image.key, &image.image_reference)
                .await?;
            publications.push(OciImagePublication {
                image,
                registry_publication,
            });
        }
        Ok(publications)
    }

    async fn get_image_publication(
        &self,
        identity: &AuthenticatedIdentity,
        id: OciImageId,
    ) -> Result<OciImagePublication, ImageCatalogError> {
        let image = self.get_image(id).await?;
        let registry_publication = self
            .platform_publication(identity, &image.key, &image.image_reference)
            .await?;
        Ok(OciImagePublication {
            image,
            registry_publication,
        })
    }
}

#[derive(Debug, FromRow)]
struct OciImageRow {
    id: Uuid,
    key: String,
    display_name: String,
    image_reference: String,
    toolchains: Value,
    architectures: Vec<String>,
    availability_state: String,
    provenance: Value,
    signature_reference: Option<String>,
    sbom_reference: Option<String>,
    platform_policy_version: String,
}

impl OciImageRow {
    fn try_into_domain(self) -> Result<OciImage, ImageCatalogError> {
        let provenance =
            serde_json::from_value::<ImageProvenance>(self.provenance).map_err(storage)?;
        let image = OciImage {
            id: OciImageId::from_uuid(self.id),
            key: ImageKey::parse(self.key).map_err(invalid_data)?,
            display_name: self.display_name,
            image_reference: OciImageReference::parse(self.image_reference)
                .map_err(invalid_data)?,
            toolchains: serde_json::from_value::<Vec<Toolchain>>(self.toolchains)
                .map_err(storage)?,
            architectures: self.architectures,
            availability: availability_state(&self.availability_state).map_err(invalid_data)?,
            provenance: ImageProvenance {
                source: provenance.source,
                signature: self.signature_reference.or(provenance.signature),
                sbom: self.sbom_reference.or(provenance.sbom),
            },
            platform_policy_version: self.platform_policy_version,
        };
        image.validate().map_err(invalid_data)?;
        Ok(image)
    }
}

#[derive(Debug, FromRow)]
struct RegistryPublicationRow {
    state: String,
    immutable_reference: String,
    architectures: Vec<String>,
    sbom_reference: Option<String>,
    provenance_reference: Option<String>,
    scan_reference: Option<String>,
    signature_reference: Option<String>,
    signature_required: bool,
}

impl RegistryPublicationRow {
    fn try_into_domain(self) -> Result<RegistryPublication, ImageCatalogValueError> {
        let state = registry_publication_state(&self.state)?;
        let signature = if self.signature_required {
            registry_evidence(self.signature_reference)?
        } else {
            if self.signature_reference.is_some() {
                return Err(ImageCatalogValueError::InvalidRegistryPublication);
            }
            RegistryEvidence::not_required()
        };
        let publication = RegistryPublication {
            state,
            availability: registry_availability(state),
            immutable_reference: Some(OciImageReference::parse(self.immutable_reference)?),
            architectures: self.architectures,
            sbom: registry_evidence(self.sbom_reference)?,
            provenance: registry_evidence(self.provenance_reference)?,
            scan: registry_evidence(self.scan_reference)?,
            signature,
        };
        publication.validate()?;
        Ok(publication)
    }
}

fn availability_state(value: &str) -> Result<AvailabilityState, ImageCatalogValueError> {
    match value {
        "available" => Ok(AvailabilityState::Available),
        "unavailable" => Ok(AvailabilityState::Unavailable),
        "retired" => Ok(AvailabilityState::Retired),
        _ => Err(ImageCatalogValueError::InvalidStoredValue),
    }
}

fn registry_publication_state(
    value: &str,
) -> Result<RegistryPublicationState, ImageCatalogValueError> {
    match value {
        "pending" => Ok(RegistryPublicationState::Pending),
        "publishing" => Ok(RegistryPublicationState::Publishing),
        "verified" => Ok(RegistryPublicationState::Verified),
        "approved" => Ok(RegistryPublicationState::Approved),
        "missing" => Ok(RegistryPublicationState::Missing),
        "retired" => Ok(RegistryPublicationState::Retired),
        _ => Err(ImageCatalogValueError::InvalidRegistryPublication),
    }
}

const fn registry_availability(state: RegistryPublicationState) -> RegistryAvailabilityState {
    match state {
        RegistryPublicationState::Approved => RegistryAvailabilityState::Available,
        RegistryPublicationState::Retired => RegistryAvailabilityState::Retired,
        RegistryPublicationState::Pending
        | RegistryPublicationState::Publishing
        | RegistryPublicationState::Verified
        | RegistryPublicationState::Missing => RegistryAvailabilityState::Unavailable,
    }
}

fn registry_evidence(
    reference: Option<String>,
) -> Result<RegistryEvidence, ImageCatalogValueError> {
    reference
        .map(OciImageReference::parse)
        .transpose()
        .map(|reference| {
            reference.map_or_else(RegistryEvidence::pending, RegistryEvidence::verified)
        })
}

const fn invalid_data(error: ImageCatalogValueError) -> ImageCatalogError {
    ImageCatalogError::InvalidData(error)
}
fn storage(error: impl std::error::Error + Send + Sync + 'static) -> ImageCatalogError {
    ImageCatalogError::Storage(Box::new(error))
}
