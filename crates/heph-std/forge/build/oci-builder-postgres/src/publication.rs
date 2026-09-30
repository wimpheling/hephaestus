use async_trait::async_trait;
use builder_catalog_domain::OciImageId;
use heph_build::{
    OciWorkerError, RepositoryOciImagePublicationLease, RepositoryOciImagePublicationStore,
};
use registry_domain::{
    ImmutableManifestReference, NamespaceClaim, OciDescriptor, PolicyVersion, PublicationIntent,
    PublicationIntentId, PublicationState, RegistryAuthority, RegistryNamespace, SupplyChainPolicy,
    VerifiedPublication,
};
use registry_postgres::PgRegistryStore;
use sqlx::PgPool;
use uuid::Uuid;

/// Durable repository-image registry lifecycle adapter.
///
/// This adapter derives the Zot namespace solely from the durable project and
/// image identifiers, then delegates immutable lifecycle transitions to the
/// registry control plane. It never accepts a repository path from a build.
#[derive(Clone)]
pub struct PgRepositoryOciImagePublicationStore {
    pool: PgPool,
    registry: PgRegistryStore,
    authority: RegistryAuthority,
    policy_version: PolicyVersion,
    supply_chain_policy: SupplyChainPolicy,
}

impl PgRepositoryOciImagePublicationStore {
    /// Creates the adapter with fixed forge registry policy.
    #[must_use]
    pub const fn new(
        pool: PgPool,
        registry: PgRegistryStore,
        authority: RegistryAuthority,
        policy_version: PolicyVersion,
        supply_chain_policy: SupplyChainPolicy,
    ) -> Self {
        Self {
            pool,
            registry,
            authority,
            policy_version,
            supply_chain_policy,
        }
    }

    async fn assert_producing_owner(
        &self,
        project_id: Uuid,
        image_id: OciImageId,
    ) -> Result<(), OciWorkerError> {
        let owned = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (
                SELECT 1 FROM repository_oci_image_definitions
                 WHERE id = $1 AND project_id = $2 AND status = 'producing'
             )",
        )
        .bind(image_id.as_uuid())
        .bind(project_id)
        .fetch_one(&self.pool)
        .await
        .map_err(|_| OciWorkerError::RegistryPublication)?;
        owned
            .then_some(())
            .ok_or(OciWorkerError::RegistryPublication)
    }

    fn intent(
        &self,
        project_id: Uuid,
        image_id: OciImageId,
        expected_manifest: OciDescriptor,
    ) -> Result<PublicationIntent, OciWorkerError> {
        let claim = repository_image_claim(project_id, image_id)?;
        let reference = ImmutableManifestReference::new(
            self.authority.clone(),
            claim.namespace().clone(),
            expected_manifest.digest().clone(),
        );
        PublicationIntent::new(
            PublicationIntentId::new(),
            claim,
            reference,
            expected_manifest,
            self.policy_version.clone(),
            self.supply_chain_policy,
        )
        .map_err(|_| OciWorkerError::RegistryPublication)
    }
}

pub fn repository_image_claim(
    project_id: Uuid,
    image_id: OciImageId,
) -> Result<NamespaceClaim, OciWorkerError> {
    let namespace = RegistryNamespace::parse(format!(
        "projects/{project_id}/repository-images/{image_id}"
    ))
    .map_err(|_| OciWorkerError::RegistryPublication)?;
    Ok(NamespaceClaim::new(namespace.owner().clone()))
}
#[async_trait]
impl RepositoryOciImagePublicationStore for PgRepositoryOciImagePublicationStore {
    async fn begin_repository_image_publication(
        &self,
        project_id: Uuid,
        image_id: OciImageId,
        expected_manifest: OciDescriptor,
    ) -> Result<RepositoryOciImagePublicationLease, OciWorkerError> {
        self.assert_producing_owner(project_id, image_id).await?;
        let requested = self.intent(project_id, image_id, expected_manifest)?;
        let stored = self
            .registry
            .create_intent(&requested)
            .await
            .map_err(|_| OciWorkerError::RegistryPublication)?;
        match stored.state() {
            PublicationState::Pending | PublicationState::Publishing => {
                let publishing = self
                    .registry
                    .begin_publishing(stored.id())
                    .await
                    .map_err(|_| OciWorkerError::RegistryPublication)?;
                Ok(RepositoryOciImagePublicationLease::Publish(publishing))
            }
            PublicationState::Verified | PublicationState::Approved => {
                Ok(RepositoryOciImagePublicationLease::Approved(stored))
            }
            PublicationState::Retired | PublicationState::Missing => {
                Err(OciWorkerError::RegistryPublication)
            }
        }
    }

    async fn record_verified_and_approve(
        &self,
        intent_id: PublicationIntentId,
        verification: VerifiedPublication,
    ) -> Result<PublicationIntent, OciWorkerError> {
        let verified = self
            .registry
            .record_verified(intent_id, verification)
            .await
            .map_err(|_| OciWorkerError::RegistryPublication)?;
        if !matches!(
            verified.state(),
            PublicationState::Verified | PublicationState::Approved
        ) {
            return Err(OciWorkerError::RegistryPublication);
        }
        self.registry
            .approve(intent_id)
            .await
            .map_err(|_| OciWorkerError::RegistryPublication)
    }

    async fn retry_repository_image_publication(
        &self,
        intent_id: PublicationIntentId,
    ) -> Result<(), OciWorkerError> {
        self.registry
            .retry(intent_id)
            .await
            .map(|_| ())
            .map_err(|_| OciWorkerError::RegistryPublication)
    }
}
