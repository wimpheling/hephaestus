//! One backend prepared-instance import, with actual SQL service success.
//!
//! This module does not enable named dispatch, invoke a VM or verify a backing.
//! A cancelled or unknown call preserves the original active claim. Manager
//! removal recovery requires the separate durable107 lineage reader.

mod completion;
mod converter;
mod errors;
mod execution;
mod validation;

pub use converter::to_import_command;
pub use errors::{InstanceImportError, InstanceImportFailure};

use std::sync::Arc;

use capability_domain::CapabilitySlotKey;
use identity_domain::AuthenticatedIdentity;
use recipe_application::{CommandIdentity, DeploymentId, ResourceProgress};
use recipe_postgres::{
    PostgresDeploymentRepository, PostgresEffectVerificationRecorder,
    PostgresExecutionPreparationRepository,
};
use release_postgres::ReleaseService;

/// Concrete owning composition with real actor preparation, release and worker pools.
///
/// None of these adapters accepts a fabricated historic authenticated identity.
/// The worker recorder is used only after this service observes the actual
/// successful release call with the exact prepared input and returned instance.
pub struct PreparedInstanceImporter {
    preparation: Arc<PostgresExecutionPreparationRepository>,
    deployment: Arc<PostgresDeploymentRepository>,
    release: Arc<ReleaseService>,
    recorder: Arc<PostgresEffectVerificationRecorder>,
}

impl PreparedInstanceImporter {
    /// Injects canonical actor adapters and a separately configured worker recorder.
    #[must_use]
    pub const fn new(
        preparation: Arc<PostgresExecutionPreparationRepository>,
        deployment: Arc<PostgresDeploymentRepository>,
        release: Arc<ReleaseService>,
        recorder: Arc<PostgresEffectVerificationRecorder>,
    ) -> Self {
        Self {
            preparation,
            deployment,
            release,
            recorder,
        }
    }

    /// Imports one frozen instance through the real release command service.
    ///
    /// Whole-graph preparation and fresh original authority precede claims and IO.
    /// A completed protected result replays without another service call. Active
    /// claims preserve their original request/generation across real service replay.
    /// No repository transaction spans the service call. Cancellation does not
    /// clear a claim or prove absence; unknown outcomes require fresh authorized retry.
    ///
    /// # Errors
    /// Rejects live denial, drift, wrong resource/command and contradictory claims.
    /// Service/recording/completion failures retain their primary error and keep
    /// the original claim held, even if permission loss prevents ambiguity recording.
    pub async fn import_prepared_instance(
        &self,
        identity: &AuthenticatedIdentity,
        original_install: CommandIdentity,
        deployment: DeploymentId,
        resource: &CapabilitySlotKey,
    ) -> Result<ResourceProgress, InstanceImportError> {
        execution::import(self, identity, original_install, deployment, resource).await
    }
}
