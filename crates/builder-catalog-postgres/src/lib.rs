//! `PostgreSQL` adapter for the platform-owned builder image catalog.

use async_trait::async_trait;
use builder_catalog_application::{
    BuilderCatalog, BuilderCatalogError, ProjectBuilderStore, ProjectBuilderStoreError,
    RegistryPublicationCatalog,
};
use builder_catalog_domain::{
    AvailabilityState, BuildNetworkPolicy, BuilderCatalogValueError, BuilderImage, BuilderImageId,
    BuilderImagePublication, BuilderImageReference, BuilderKey, BuilderProvenance,
    BuilderSourcePath, DependencyPolicy, NewProjectBuilder, OciDigest, PreparationState,
    ProjectBuilderDefinition, ProjectBuilderId, ProjectBuilderProvenance,
    ProjectBuilderPublication, ProjectBuilderStatus, RegistryAvailabilityState, RegistryEvidence,
    RegistryPublication, RegistryPublicationState, Toolchain,
};
use identity_domain::AuthenticatedIdentity;
use serde_json::Value;
use sqlx::{FromRow, PgPool, Postgres, Transaction};
use uuid::Uuid;

/// PostgreSQL-backed builder catalog.
#[derive(Clone)]
pub struct PgBuilderCatalog {
    pool: PgPool,
}

impl PgBuilderCatalog {
    /// Creates an adapter over the application database pool.
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    async fn platform_publication(
        &self,
        identity: &AuthenticatedIdentity,
        key: &BuilderKey,
        reference: &BuilderImageReference,
        preparation: PreparationState,
    ) -> Result<RegistryPublication, BuilderCatalogError> {
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
             WHERE publication.owner_kind = 'platform_builder'
               AND publication.platform_builder_key = $1
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
            .map_err(invalid_data)
            .map(|publication| {
                publication.unwrap_or_else(|| fallback_platform_publication(preparation))
            })
    }

    async fn project_publication(
        &self,
        identity: &AuthenticatedIdentity,
        builder: &ProjectBuilderDefinition,
    ) -> Result<RegistryPublication, ProjectBuilderStoreError> {
        let mut transaction = authz_postgres::begin_actor_transaction(&self.pool, identity)
            .await
            .map_err(storage_project_builder)?;
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
             WHERE publication.owner_kind = 'repository_builder'
               AND publication.project_id = $1 AND publication.owner_id = $2
               AND ($3::text IS NULL OR publication.registry_authority || '/'
                    || namespace.repository_path || '@' || publication.expected_digest = $3)
             ORDER BY publication.created_at DESC, publication.id DESC
             LIMIT 1",
        )
        .bind(builder.project_id)
        .bind(builder.id.as_uuid())
        .bind(builder.oci_image_reference.as_ref().map(BuilderImageReference::as_str))
        .fetch_optional(&mut *transaction)
        .await
        .map_err(storage_project_builder)?;
        transaction
            .commit()
            .await
            .map_err(storage_project_builder)?;
        row.map(RegistryPublicationRow::try_into_domain)
            .transpose()
            .map_err(invalid_project_builder_data)
            .map(|publication| {
                publication.unwrap_or_else(|| fallback_project_publication(builder.status))
            })
    }
}

#[async_trait]
impl BuilderCatalog for PgBuilderCatalog {
    async fn list_builder_images(&self) -> Result<Vec<BuilderImage>, BuilderCatalogError> {
        let rows = sqlx::query_as::<_, BuilderImageRow>(
            "SELECT id, key, display_name, image_reference, toolchains,
                    architectures, preparation_state, availability_state,
                    network_ceiling, max_vcpus, max_memory_mib,
                    dependency_policy, provenance, signature_reference,
                    sbom_reference, platform_policy_version
             FROM builder_images
             ORDER BY key, id",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(storage)?;
        rows.into_iter()
            .map(BuilderImageRow::try_into_domain)
            .collect()
    }

    async fn get_builder_image(
        &self,
        id: BuilderImageId,
    ) -> Result<BuilderImage, BuilderCatalogError> {
        let row = sqlx::query_as::<_, BuilderImageRow>(
            "SELECT id, key, display_name, image_reference, toolchains,
                    architectures, preparation_state, availability_state,
                    network_ceiling, max_vcpus, max_memory_mib,
                    dependency_policy, provenance, signature_reference,
                    sbom_reference, platform_policy_version
             FROM builder_images WHERE id = $1",
        )
        .bind(id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(storage)?
        .ok_or(BuilderCatalogError::NotFound)?;
        row.try_into_domain()
    }

    async fn find_builder_image_by_reference(
        &self,
        reference: &BuilderImageReference,
    ) -> Result<BuilderImage, BuilderCatalogError> {
        let row = sqlx::query_as::<_, BuilderImageRow>(
            "SELECT id, key, display_name, image_reference, toolchains,
                    architectures, preparation_state, availability_state,
                    network_ceiling, max_vcpus, max_memory_mib,
                    dependency_policy, provenance, signature_reference,
                    sbom_reference, platform_policy_version
             FROM builder_images WHERE image_reference = $1",
        )
        .bind(reference.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(storage)?
        .ok_or(BuilderCatalogError::NotFound)?;
        row.try_into_domain()
    }
}

#[async_trait]
impl RegistryPublicationCatalog for PgBuilderCatalog {
    async fn list_builder_image_publications(
        &self,
        identity: &AuthenticatedIdentity,
    ) -> Result<Vec<BuilderImagePublication>, BuilderCatalogError> {
        let images = BuilderCatalog::list_builder_images(self).await?;
        let mut publications = Vec::with_capacity(images.len());
        for image in images {
            let registry_publication = self
                .platform_publication(
                    identity,
                    &image.key,
                    &image.image_reference,
                    image.preparation,
                )
                .await?;
            publications.push(BuilderImagePublication {
                image,
                registry_publication,
            });
        }
        Ok(publications)
    }

    async fn get_builder_image_publication(
        &self,
        identity: &AuthenticatedIdentity,
        id: BuilderImageId,
    ) -> Result<BuilderImagePublication, BuilderCatalogError> {
        let image = BuilderCatalog::get_builder_image(self, id).await?;
        let registry_publication = self
            .platform_publication(
                identity,
                &image.key,
                &image.image_reference,
                image.preparation,
            )
            .await?;
        Ok(BuilderImagePublication {
            image,
            registry_publication,
        })
    }

    async fn list_project_builder_publications(
        &self,
        identity: &AuthenticatedIdentity,
        project_id: Uuid,
    ) -> Result<Vec<ProjectBuilderPublication>, ProjectBuilderStoreError> {
        let builders = ProjectBuilderStore::list_project_builders(self, project_id).await?;
        let mut publications = Vec::with_capacity(builders.len());
        for builder in builders {
            let registry_publication = self.project_publication(identity, &builder).await?;
            publications.push(ProjectBuilderPublication {
                builder,
                registry_publication,
            });
        }
        Ok(publications)
    }

    async fn get_project_builder_publication(
        &self,
        identity: &AuthenticatedIdentity,
        project_id: Uuid,
        id: ProjectBuilderId,
    ) -> Result<ProjectBuilderPublication, ProjectBuilderStoreError> {
        let builder = ProjectBuilderStore::get_project_builder(self, project_id, id).await?;
        let registry_publication = self.project_publication(identity, &builder).await?;
        Ok(ProjectBuilderPublication {
            builder,
            registry_publication,
        })
    }
}

#[async_trait]
impl ProjectBuilderStore for PgBuilderCatalog {
    async fn create_project_builder(
        &self,
        builder: NewProjectBuilder,
    ) -> Result<ProjectBuilderDefinition, ProjectBuilderStoreError> {
        builder
            .validate()
            .map_err(ProjectBuilderStoreError::InvalidData)?;
        let project_id = builder.project_id;
        let builder_id = builder.id;
        let mut transaction = self.pool.begin().await.map_err(storage_project_builder)?;
        let row = sqlx::query_as::<_, ProjectBuilderRow>(
            "INSERT INTO project_builder_definitions
                (id, project_id, source_repository_id, key, display_name,
                 source_revision, dockerfile_path, context_path, context_digest,
                 approved_base_image_reference, status)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, 'draft')
             RETURNING id, project_id, source_repository_id, key, display_name,
                 source_revision, dockerfile_path, context_path, context_digest,
                 approved_base_image_reference, status, oci_image_reference,
                 oci_image_digest, provenance, failure_reason, created_at, updated_at",
        )
        .bind(builder.id.as_uuid())
        .bind(builder.project_id)
        .bind(builder.source_repository_id)
        .bind(builder.key.as_str())
        .bind(builder.display_name)
        .bind(builder.source_revision)
        .bind(builder.dockerfile_path.as_str())
        .bind(builder.context_path.as_str())
        .bind(builder.context_digest.as_str())
        .bind(builder.approved_base_image.as_str())
        .fetch_one(&mut *transaction)
        .await
        .map_err(storage_project_builder)?;
        append_project_builder_event(&mut transaction, project_id, builder_id, "created", "draft")
            .await?;
        transaction
            .commit()
            .await
            .map_err(storage_project_builder)?;
        row.try_into_domain()
    }

    async fn list_project_builders(
        &self,
        project_id: Uuid,
    ) -> Result<Vec<ProjectBuilderDefinition>, ProjectBuilderStoreError> {
        let rows = sqlx::query_as::<_, ProjectBuilderRow>(
            "SELECT id, project_id, source_repository_id, key, display_name,
                    source_revision, dockerfile_path, context_path, context_digest,
                    approved_base_image_reference, status, oci_image_reference,
                    oci_image_digest, provenance, failure_reason, created_at, updated_at
             FROM project_builder_definitions
             WHERE project_id = $1
             ORDER BY key, id",
        )
        .bind(project_id)
        .fetch_all(&self.pool)
        .await
        .map_err(storage_project_builder)?;
        rows.into_iter()
            .map(ProjectBuilderRow::try_into_domain)
            .collect()
    }

    async fn get_project_builder(
        &self,
        project_id: Uuid,
        id: ProjectBuilderId,
    ) -> Result<ProjectBuilderDefinition, ProjectBuilderStoreError> {
        let mut transaction = self.pool.begin().await.map_err(storage_project_builder)?;
        let row = sqlx::query_as::<_, ProjectBuilderRow>(
            "SELECT id, project_id, source_repository_id, key, display_name,
                    source_revision, dockerfile_path, context_path, context_digest,
                    approved_base_image_reference, status, oci_image_reference,
                    oci_image_digest, provenance, failure_reason, created_at, updated_at
             FROM project_builder_definitions
             WHERE project_id = $1 AND id = $2",
        )
        .bind(project_id)
        .bind(id.as_uuid())
        .fetch_optional(&mut *transaction)
        .await
        .map_err(storage_project_builder)?
        .ok_or(ProjectBuilderStoreError::NotFound)?;
        row.try_into_domain()
    }

    async fn get_project_builder_by_repository_key(
        &self,
        project_id: Uuid,
        source_repository_id: Uuid,
        key: &str,
        source_revision: &str,
    ) -> Result<ProjectBuilderDefinition, ProjectBuilderStoreError> {
        let row = sqlx::query_as::<_, ProjectBuilderRow>(
            "SELECT id, project_id, source_repository_id, key, display_name,
                    source_revision, dockerfile_path, context_path, context_digest,
                    approved_base_image_reference, status, oci_image_reference,
                    oci_image_digest, provenance, failure_reason, created_at, updated_at
             FROM project_builder_definitions
             WHERE project_id = $1 AND source_repository_id = $2 AND key = $3
               AND source_revision = $4
             ORDER BY created_at DESC, id DESC
             LIMIT 1",
        )
        .bind(project_id)
        .bind(source_repository_id)
        .bind(key)
        .bind(source_revision)
        .fetch_optional(&self.pool)
        .await
        .map_err(storage_project_builder)?
        .ok_or(ProjectBuilderStoreError::NotFound)?;
        row.try_into_domain()
    }

    async fn begin_project_builder_preparation(
        &self,
        project_id: Uuid,
        id: ProjectBuilderId,
    ) -> Result<ProjectBuilderDefinition, ProjectBuilderStoreError> {
        let mut transaction = self.pool.begin().await.map_err(storage_project_builder)?;
        let row = sqlx::query_as::<_, ProjectBuilderRow>(
            "UPDATE project_builder_definitions
             SET status = 'preparing', failure_reason = NULL, updated_at = now()
             WHERE project_id = $1 AND id = $2 AND status IN ('draft', 'failed')
             RETURNING id, project_id, source_repository_id, key, display_name,
                 source_revision, dockerfile_path, context_path, context_digest,
                 approved_base_image_reference, status, oci_image_reference,
                 oci_image_digest, provenance, failure_reason, created_at, updated_at",
        )
        .bind(project_id)
        .bind(id.as_uuid())
        .fetch_optional(&mut *transaction)
        .await
        .map_err(storage_project_builder)?;
        self.lifecycle_row_or_error(transaction, row, project_id, id, "preparing")
            .await
    }

    async fn complete_project_builder(
        &self,
        project_id: Uuid,
        id: ProjectBuilderId,
        output_reference: BuilderImageReference,
        output_digest: OciDigest,
        provenance: ProjectBuilderProvenance,
    ) -> Result<ProjectBuilderDefinition, ProjectBuilderStoreError> {
        let reference_digest = output_reference
            .digest()
            .map_err(ProjectBuilderStoreError::InvalidData)?;
        if reference_digest != output_digest {
            return Err(ProjectBuilderStoreError::InvalidData(
                BuilderCatalogValueError::InvalidProjectBuilderState,
            ));
        }
        provenance
            .validate()
            .map_err(ProjectBuilderStoreError::InvalidData)?;
        let provenance = serde_json::to_value(provenance).map_err(storage_project_builder)?;
        let mut transaction = self.pool.begin().await.map_err(storage_project_builder)?;
        let row = sqlx::query_as::<_, ProjectBuilderRow>(
            "UPDATE project_builder_definitions
             SET status = 'ready', oci_image_reference = $3, oci_image_digest = $4,
                 provenance = $5, failure_reason = NULL, updated_at = now()
             WHERE project_id = $1 AND id = $2 AND status = 'preparing'
             RETURNING id, project_id, source_repository_id, key, display_name,
                 source_revision, dockerfile_path, context_path, context_digest,
                 approved_base_image_reference, status, oci_image_reference,
                 oci_image_digest, provenance, failure_reason, created_at, updated_at",
        )
        .bind(project_id)
        .bind(id.as_uuid())
        .bind(output_reference.as_str())
        .bind(output_digest.as_str())
        .bind(provenance)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(storage_project_builder)?;
        self.lifecycle_row_or_error(transaction, row, project_id, id, "ready")
            .await
    }

    async fn fail_project_builder(
        &self,
        project_id: Uuid,
        id: ProjectBuilderId,
        reason: String,
    ) -> Result<ProjectBuilderDefinition, ProjectBuilderStoreError> {
        if reason.trim().is_empty() || reason.len() > 2048 {
            return Err(ProjectBuilderStoreError::InvalidData(
                BuilderCatalogValueError::InvalidFailureReason,
            ));
        }
        let mut transaction = self.pool.begin().await.map_err(storage_project_builder)?;
        let row = sqlx::query_as::<_, ProjectBuilderRow>(
            "UPDATE project_builder_definitions
             SET status = 'failed', failure_reason = $3, updated_at = now()
             WHERE project_id = $1 AND id = $2 AND status = 'preparing'
             RETURNING id, project_id, source_repository_id, key, display_name,
                 source_revision, dockerfile_path, context_path, context_digest,
                 approved_base_image_reference, status, oci_image_reference,
                 oci_image_digest, provenance, failure_reason, created_at, updated_at",
        )
        .bind(project_id)
        .bind(id.as_uuid())
        .bind(reason)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(storage_project_builder)?;
        self.lifecycle_row_or_error(transaction, row, project_id, id, "failed")
            .await
    }

    async fn retire_project_builder(
        &self,
        project_id: Uuid,
        id: ProjectBuilderId,
    ) -> Result<ProjectBuilderDefinition, ProjectBuilderStoreError> {
        let mut transaction = self.pool.begin().await.map_err(storage_project_builder)?;
        let row = sqlx::query_as::<_, ProjectBuilderRow>(
            "UPDATE project_builder_definitions
             SET status = 'retired', updated_at = now()
             WHERE project_id = $1 AND id = $2 AND status <> 'retired'
             RETURNING id, project_id, source_repository_id, key, display_name,
                 source_revision, dockerfile_path, context_path, context_digest,
                 approved_base_image_reference, status, oci_image_reference,
                 oci_image_digest, provenance, failure_reason, created_at, updated_at",
        )
        .bind(project_id)
        .bind(id.as_uuid())
        .fetch_optional(&mut *transaction)
        .await
        .map_err(storage_project_builder)?;
        self.lifecycle_row_or_error(transaction, row, project_id, id, "retired")
            .await
    }
}

async fn append_project_builder_event(
    transaction: &mut Transaction<'_, Postgres>,
    project_id: Uuid,
    builder_id: ProjectBuilderId,
    change_kind: &str,
    status: &str,
) -> Result<(), ProjectBuilderStoreError> {
    sqlx::query(
        "SELECT event_id FROM append_application_event(
            gen_random_uuid(), 'project', $1, 'project', $1,
            'project.changed', $2, $3, $4, NULL
        )",
    )
    .bind(project_id)
    .bind(change_kind)
    .bind(status)
    .bind(builder_id.as_uuid())
    .execute(&mut **transaction)
    .await
    .map_err(storage_project_builder)?;
    Ok(())
}

impl PgBuilderCatalog {
    async fn lifecycle_row_or_error(
        &self,
        mut transaction: Transaction<'_, Postgres>,
        row: Option<ProjectBuilderRow>,
        project_id: Uuid,
        id: ProjectBuilderId,
        status: &str,
    ) -> Result<ProjectBuilderDefinition, ProjectBuilderStoreError> {
        if let Some(row) = row {
            append_project_builder_event(&mut transaction, project_id, id, "state_changed", status)
                .await?;
            transaction
                .commit()
                .await
                .map_err(storage_project_builder)?;
            row.try_into_domain()
        } else {
            let exists: bool = sqlx::query_scalar(
                "SELECT EXISTS(
                    SELECT 1 FROM project_builder_definitions
                    WHERE project_id = $1 AND id = $2
                )",
            )
            .bind(project_id)
            .bind(id.as_uuid())
            .fetch_one(&mut *transaction)
            .await
            .map_err(storage_project_builder)?;
            Err(if exists {
                ProjectBuilderStoreError::Conflict
            } else {
                ProjectBuilderStoreError::NotFound
            })
        }
    }
}

#[derive(Debug, FromRow)]
struct ProjectBuilderRow {
    id: Uuid,
    project_id: Uuid,
    source_repository_id: Uuid,
    key: String,
    display_name: String,
    source_revision: String,
    dockerfile_path: String,
    context_path: String,
    context_digest: String,
    approved_base_image_reference: String,
    status: String,
    oci_image_reference: Option<String>,
    oci_image_digest: Option<String>,
    provenance: Option<Value>,
    failure_reason: Option<String>,
    created_at: time::OffsetDateTime,
    updated_at: time::OffsetDateTime,
}

impl ProjectBuilderRow {
    fn try_into_domain(self) -> Result<ProjectBuilderDefinition, ProjectBuilderStoreError> {
        let builder = ProjectBuilderDefinition {
            id: ProjectBuilderId::from_uuid(self.id),
            project_id: self.project_id,
            key: BuilderKey::parse(self.key).map_err(invalid_project_builder_data)?,
            display_name: self.display_name,
            source_repository_id: self.source_repository_id,
            source_revision: self.source_revision,
            dockerfile_path: BuilderSourcePath::parse(self.dockerfile_path)
                .map_err(invalid_project_builder_data)?,
            context_path: BuilderSourcePath::parse(self.context_path)
                .map_err(invalid_project_builder_data)?,
            context_digest: OciDigest::parse(self.context_digest)
                .map_err(invalid_project_builder_data)?,
            approved_base_image: BuilderImageReference::parse(self.approved_base_image_reference)
                .map_err(invalid_project_builder_data)?,
            status: project_builder_status(&self.status).map_err(invalid_project_builder_data)?,
            oci_image_reference: self
                .oci_image_reference
                .map(BuilderImageReference::parse)
                .transpose()
                .map_err(invalid_project_builder_data)?,
            oci_image_digest: self
                .oci_image_digest
                .map(OciDigest::parse)
                .transpose()
                .map_err(invalid_project_builder_data)?,
            provenance: self
                .provenance
                .map(serde_json::from_value)
                .transpose()
                .map_err(storage_project_builder)?,
            failure_reason: self.failure_reason,
            created_at: self.created_at,
            updated_at: self.updated_at,
        };
        builder.validate().map_err(invalid_project_builder_data)?;
        Ok(builder)
    }
}

#[derive(Debug, FromRow)]
struct BuilderImageRow {
    id: Uuid,
    key: String,
    display_name: String,
    image_reference: String,
    toolchains: Value,
    architectures: Vec<String>,
    preparation_state: String,
    availability_state: String,
    network_ceiling: String,
    max_vcpus: i16,
    max_memory_mib: i32,
    dependency_policy: String,
    provenance: Value,
    signature_reference: Option<String>,
    sbom_reference: Option<String>,
    platform_policy_version: String,
}

impl BuilderImageRow {
    fn try_into_domain(self) -> Result<BuilderImage, BuilderCatalogError> {
        let key = BuilderKey::parse(self.key).map_err(invalid_data)?;
        let image_reference =
            BuilderImageReference::parse(self.image_reference).map_err(invalid_data)?;
        let toolchains =
            serde_json::from_value::<Vec<Toolchain>>(self.toolchains).map_err(storage)?;
        let provenance =
            serde_json::from_value::<BuilderProvenance>(self.provenance).map_err(storage)?;
        let preparation = preparation_state(&self.preparation_state).map_err(invalid_data)?;
        let availability = availability_state(&self.availability_state).map_err(invalid_data)?;
        let network_ceiling = network_policy(&self.network_ceiling).map_err(invalid_data)?;
        let dependency_policy = dependency_policy(&self.dependency_policy).map_err(invalid_data)?;
        let max_vcpus = u8::try_from(self.max_vcpus)
            .map_err(|_| invalid_data(BuilderCatalogValueError::InvalidResourceCeiling))?;
        let max_memory_mib = u32::try_from(self.max_memory_mib)
            .map_err(|_| invalid_data(BuilderCatalogValueError::InvalidResourceCeiling))?;
        let entry = BuilderImage {
            id: BuilderImageId::from_uuid(self.id),
            key,
            display_name: self.display_name,
            image_reference,
            toolchains,
            architectures: self.architectures,
            preparation,
            availability,
            network_ceiling,
            max_vcpus,
            max_memory_mib,
            dependency_policy,
            provenance: BuilderProvenance {
                source: provenance.source,
                signature: self.signature_reference.or(provenance.signature),
                sbom: self.sbom_reference.or(provenance.sbom),
            },
            platform_policy_version: self.platform_policy_version,
        };
        entry.validate().map_err(invalid_data)?;
        Ok(entry)
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
    fn try_into_domain(self) -> Result<RegistryPublication, BuilderCatalogValueError> {
        let state = registry_publication_state(&self.state)?;
        let availability = registry_availability(state);
        let immutable_reference = BuilderImageReference::parse(self.immutable_reference)?;
        let sbom = registry_evidence(self.sbom_reference)?;
        let provenance = registry_evidence(self.provenance_reference)?;
        let scan = registry_evidence(self.scan_reference)?;
        let signature = if self.signature_required {
            registry_evidence(self.signature_reference)?
        } else {
            if self.signature_reference.is_some() {
                return Err(BuilderCatalogValueError::InvalidRegistryPublication);
            }
            RegistryEvidence::not_required()
        };
        let publication = RegistryPublication {
            state,
            availability,
            immutable_reference: Some(immutable_reference),
            architectures: self.architectures,
            sbom,
            provenance,
            scan,
            signature,
        };
        publication.validate()?;
        Ok(publication)
    }
}

const fn fallback_platform_publication(preparation: PreparationState) -> RegistryPublication {
    match preparation {
        PreparationState::Failed => RegistryPublication::failed(),
        PreparationState::Ready | PreparationState::Preparing => {
            RegistryPublication::not_requested()
        }
    }
}

const fn fallback_project_publication(status: ProjectBuilderStatus) -> RegistryPublication {
    match status {
        ProjectBuilderStatus::Failed => RegistryPublication::failed(),
        ProjectBuilderStatus::Draft
        | ProjectBuilderStatus::Preparing
        | ProjectBuilderStatus::Ready
        | ProjectBuilderStatus::Retired => RegistryPublication::not_requested(),
    }
}

fn registry_publication_state(
    value: &str,
) -> Result<RegistryPublicationState, BuilderCatalogValueError> {
    match value {
        "pending" => Ok(RegistryPublicationState::Pending),
        "publishing" => Ok(RegistryPublicationState::Publishing),
        "verified" => Ok(RegistryPublicationState::Verified),
        "approved" => Ok(RegistryPublicationState::Approved),
        "missing" => Ok(RegistryPublicationState::Missing),
        "retired" => Ok(RegistryPublicationState::Retired),
        _ => Err(BuilderCatalogValueError::InvalidRegistryPublication),
    }
}

const fn registry_availability(state: RegistryPublicationState) -> RegistryAvailabilityState {
    match state {
        RegistryPublicationState::Approved => RegistryAvailabilityState::Available,
        RegistryPublicationState::Retired => RegistryAvailabilityState::Retired,
        RegistryPublicationState::NotRequested
        | RegistryPublicationState::Pending
        | RegistryPublicationState::Publishing
        | RegistryPublicationState::Verified
        | RegistryPublicationState::Missing
        | RegistryPublicationState::Failed => RegistryAvailabilityState::Unavailable,
    }
}

fn registry_evidence(
    reference: Option<String>,
) -> Result<RegistryEvidence, BuilderCatalogValueError> {
    reference
        .map(BuilderImageReference::parse)
        .transpose()
        .map(|reference| {
            reference.map_or_else(RegistryEvidence::pending, RegistryEvidence::verified)
        })
}

fn preparation_state(value: &str) -> Result<PreparationState, BuilderCatalogValueError> {
    match value {
        "ready" => Ok(PreparationState::Ready),
        "preparing" => Ok(PreparationState::Preparing),
        "failed" => Ok(PreparationState::Failed),
        _ => Err(BuilderCatalogValueError::InvalidStoredValue),
    }
}

fn availability_state(value: &str) -> Result<AvailabilityState, BuilderCatalogValueError> {
    match value {
        "available" => Ok(AvailabilityState::Available),
        "unavailable" => Ok(AvailabilityState::Unavailable),
        "retired" => Ok(AvailabilityState::Retired),
        _ => Err(BuilderCatalogValueError::InvalidStoredValue),
    }
}

fn network_policy(value: &str) -> Result<BuildNetworkPolicy, BuilderCatalogValueError> {
    match value {
        "disabled" => Ok(BuildNetworkPolicy::Disabled),
        "broker_only" => Ok(BuildNetworkPolicy::BrokerOnly),
        "egress" => Ok(BuildNetworkPolicy::Egress),
        _ => Err(BuilderCatalogValueError::InvalidStoredValue),
    }
}

fn dependency_policy(value: &str) -> Result<DependencyPolicy, BuilderCatalogValueError> {
    match value {
        "vendored_offline" => Ok(DependencyPolicy::VendoredOffline),
        "read_only_platform_cache" => Ok(DependencyPolicy::ReadOnlyPlatformCache),
        "constrained_registry_egress" => Ok(DependencyPolicy::ConstrainedRegistryEgress),
        _ => Err(BuilderCatalogValueError::InvalidStoredValue),
    }
}

fn project_builder_status(value: &str) -> Result<ProjectBuilderStatus, BuilderCatalogValueError> {
    match value {
        "draft" => Ok(ProjectBuilderStatus::Draft),
        "preparing" => Ok(ProjectBuilderStatus::Preparing),
        "ready" => Ok(ProjectBuilderStatus::Ready),
        "failed" => Ok(ProjectBuilderStatus::Failed),
        "retired" => Ok(ProjectBuilderStatus::Retired),
        _ => Err(BuilderCatalogValueError::InvalidStoredValue),
    }
}

const fn invalid_data(error: BuilderCatalogValueError) -> BuilderCatalogError {
    BuilderCatalogError::InvalidData(error)
}

fn storage(error: impl std::error::Error + Send + Sync + 'static) -> BuilderCatalogError {
    BuilderCatalogError::Storage(Box::new(error))
}

const fn invalid_project_builder_data(error: BuilderCatalogValueError) -> ProjectBuilderStoreError {
    ProjectBuilderStoreError::InvalidData(error)
}

fn storage_project_builder(
    error: impl std::error::Error + Send + Sync + 'static,
) -> ProjectBuilderStoreError {
    ProjectBuilderStoreError::Storage(Box::new(error))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft_row() -> ProjectBuilderRow {
        ProjectBuilderRow {
            id: Uuid::new_v4(),
            project_id: Uuid::new_v4(),
            source_repository_id: Uuid::new_v4(),
            key: String::from("custom"),
            display_name: String::from("Custom builder"),
            source_revision: String::from("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
            dockerfile_path: String::from("Dockerfile.builder"),
            context_path: String::from("."),
            context_digest: String::from(
                "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            ),
            approved_base_image_reference: String::from(
                "registry.example/ubuntu@sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
            ),
            status: String::from("draft"),
            oci_image_reference: None,
            oci_image_digest: None,
            provenance: None,
            failure_reason: None,
            created_at: time::OffsetDateTime::UNIX_EPOCH,
            updated_at: time::OffsetDateTime::UNIX_EPOCH,
        }
    }

    #[test]
    fn maps_valid_project_builder_rows() {
        let builder = draft_row()
            .try_into_domain()
            .expect("valid project builder row");
        assert_eq!(builder.status, ProjectBuilderStatus::Draft);
        assert_eq!(builder.context_path.as_str(), ".");
    }

    #[test]
    fn rejects_persisted_output_digest_mismatch() {
        let mut row = draft_row();
        row.status = String::from("ready");
        row.oci_image_reference = Some(String::from(
            "registry.example/custom@sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
        ));
        row.oci_image_digest = Some(String::from(
            "sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
        ));
        row.provenance = Some(serde_json::json!({
            "source_revision": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "context_digest": "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "attestation_reference": "attestation://test"
        }));
        assert!(matches!(
            row.try_into_domain(),
            Err(ProjectBuilderStoreError::InvalidData(
                BuilderCatalogValueError::InvalidProjectBuilderState
            ))
        ));
    }
}
