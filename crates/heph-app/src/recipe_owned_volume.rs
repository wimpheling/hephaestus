//! One prepared owned-volume effect; no provider root bootstrap or whole engine.

mod actor;
mod completion;
mod errors;
mod execution;
mod validation;

pub use errors::{OwnedVolumeCreateError, OwnedVolumeCreateFailure};

use capability_domain::CapabilitySlotKey;
use identity_domain::AuthenticatedIdentity;
use recipe_application::{CommandIdentity, DeploymentId, ResourceProgress};
use recipe_postgres::{
    PostgresDeploymentRepository, PostgresEffectVerificationRecorder,
    PostgresExecutionPreparationRepository,
};
use std::sync::Arc;
use volume_local::{LocalVolumeConfig, LocalVolumeStore, OwnedVolumeMetadata, VolumeRootOwner};
use volume_postgres::PostgresVolumeMetadataRepository;
use volume_trait::VolumeError;

/// Owning backend composition for one positive original Install Volume/Create.
///
/// The actual persistent root owner must already exist. This service never
/// initializes roots, reconstructs an actor, acquires runtime leases or selects a
/// new operation/fence after uncertainty. Metadata Ready does not complete106.
#[derive(Clone)]
pub struct PreparedOwnedVolumeCreator {
    preparation: Arc<PostgresExecutionPreparationRepository>,
    deployment: Arc<PostgresDeploymentRepository>,
    actor: Arc<PostgresVolumeMetadataRepository>,
    worker: Arc<PostgresVolumeMetadataRepository>,
    recorder: Arc<PostgresEffectVerificationRecorder>,
    local: LocalVolumeStore,
    owner: Arc<VolumeRootOwner>,
}
impl PreparedOwnedVolumeCreator {
    /// Validates actual existing root/host ownership and configures actor/worker adapters.
    ///
    /// No root initialization or discovery is performed. The caller supplies its
    /// trusted static Local configuration; physical and recipe namespaces differ.
    ///
    /// # Errors
    /// Invalid configuration, replaced/unclassified roots and scope mismatch deny.
    pub fn new(
        preparation: Arc<PostgresExecutionPreparationRepository>,
        deployment: Arc<PostgresDeploymentRepository>,
        actor: Arc<PostgresVolumeMetadataRepository>,
        worker: Arc<PostgresVolumeMetadataRepository>,
        recorder: Arc<PostgresEffectVerificationRecorder>,
        config: LocalVolumeConfig,
        owner: Arc<VolumeRootOwner>,
    ) -> Result<Self, VolumeError> {
        let local = LocalVolumeStore::new(worker.clone(), config)?.with_owned_metadata(
            OwnedVolumeMetadata::new(actor.clone(), worker.clone(), owner.clone()),
        )?;
        Ok(Self {
            preparation,
            deployment,
            actor,
            worker,
            recorder,
            local,
            owner,
        })
    }
    /// Creates or resumes exactly one protected original owned volume effect.
    ///
    /// Fresh whole113/current106/config checks precede claims and each Local
    /// first-write/format admission. Discovery preserves the original111 operation
    /// and CAS. Ready-before106 retries verify the actual pinned filesystem under
    /// non-clone custody; caller cancellation cannot release that custody while
    /// completion may commit. Completed106 replay returns progress without fsck.
    ///
    /// # Errors
    /// Denied/drifted preparation, wrong resources and unknown/foreign history deny.
    /// Primary provider/completion errors survive failure to record ambiguity.
    /// Ready-removal after source withdrawal requires a separate manager-qualified
    /// removal path; this service never calls partial retention or claims absence.
    pub async fn create_prepared_volume(
        &self,
        identity: &AuthenticatedIdentity,
        original_install: CommandIdentity,
        deployment: DeploymentId,
        resource: &CapabilitySlotKey,
    ) -> Result<ResourceProgress, OwnedVolumeCreateError> {
        Box::pin(execution::create(
            self,
            identity,
            original_install,
            deployment,
            resource,
        ))
        .await
    }
}
