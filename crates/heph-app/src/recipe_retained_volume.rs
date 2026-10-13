//! Explicit retained owned-volume verification with protected native custody.

mod completion;
mod errors;
mod execution;
mod pending;
mod validation;

pub use errors::{RetainedVolumeVerificationError, RetainedVolumeVerificationFailure};

use async_trait::async_trait;
use heph_runtime::VmProvider;
use identity_domain::AuthenticatedIdentity;
use recipe_application::{EffectClaim, OwnedReadyRetentionRepository, ResourceProgress};
use recipe_postgres::{PostgresDeploymentRepository, PostgresEffectVerificationRecorder};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
use volume_local::{LocalVolumeStore, VolumeRootOwner};
use volume_postgres::PostgresVolumeMetadataRepository;
use volume_trait::{VolumeError, VolumeRetainedReuseRepository};

/// Optional backend port for an exact NEW retained-volume external claim.
///
/// Engine construction must explicitly inject this port. It grants no authority
/// to verify ordinary external volumes or create, adopt, mount or write backing.
#[async_trait]
pub trait RetainedVolumeVerifier: Send + Sync + 'static {
    /// Verifies one already admitted Install/`VerifyExternal` claim.
    ///
    /// # Errors
    /// Current denial, mismatched lineage/scope or unresolved completion remains held.
    async fn verify_retained_volume(
        &self,
        identity: &AuthenticatedIdentity,
        claim: &EffectClaim,
    ) -> Result<ResourceProgress, RetainedVolumeVerificationError>;
}

/// Actual actor, worker, Local root owner and provider composition for retained reuse.
#[derive(Clone)]
pub struct PreparedRetainedVolumeVerifier {
    actor: Arc<dyn OwnedReadyRetentionRepository>,
    worker: Arc<dyn VolumeRetainedReuseRepository>,
    metadata: Arc<PostgresVolumeMetadataRepository>,
    recorder: Arc<PostgresEffectVerificationRecorder>,
    deployment: Arc<PostgresDeploymentRepository>,
    local: Arc<LocalVolumeStore>,
    owner: Arc<VolumeRootOwner>,
    provider: Arc<dyn VmProvider>,
    pending: Arc<Mutex<BTreeMap<uuid::Uuid, pending::Control>>>,
}

/// Inspectable protected completion state; comparison data grants no authority.
#[derive(Debug, Clone)]
pub struct PendingRetainedVolumeVerification {
    /// Exact original NEW external claim held by the protected owner.
    pub claim: EffectClaim,
    /// A fresh actor reread has not established committed pair or closed noncommit.
    pub ambiguous: bool,
}
impl PreparedRetainedVolumeVerifier {
    /// Injects separated role adapters and an already configured sealed Local store.
    ///
    /// Local must carry the supplied actual persistent root owner. Original full
    /// purpose comparisons and Local admission reject any differing composition.
    /// Actor and retained worker adapters must use the provider's checked opaque
    /// VM scope; disk ownership remains a separate physical namespace.
    ///
    /// # Errors
    /// Invalid/replaced actual root ownership denies construction.
    // Each argument is an independently owning role/provider boundary.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        actor: Arc<dyn OwnedReadyRetentionRepository>,
        worker: Arc<dyn VolumeRetainedReuseRepository>,
        metadata: Arc<PostgresVolumeMetadataRepository>,
        recorder: Arc<PostgresEffectVerificationRecorder>,
        deployment: Arc<PostgresDeploymentRepository>,
        local: Arc<LocalVolumeStore>,
        owner: Arc<VolumeRootOwner>,
        provider: Arc<dyn VmProvider>,
    ) -> Result<Self, VolumeError> {
        owner.validate()?;
        Ok(Self {
            actor,
            worker,
            metadata,
            recorder,
            deployment,
            local,
            owner,
            provider,
            pending: Arc::new(Mutex::new(BTreeMap::new())),
        })
    }

    /// Lists actual protected owners, including unresolved outcomes retaining custody.
    #[must_use]
    pub fn pending_verifications(&self) -> Vec<PendingRetainedVolumeVerification> {
        pending::inspect(self)
    }
}
#[async_trait]
impl RetainedVolumeVerifier for PreparedRetainedVolumeVerifier {
    /// Exact committed pair replay precedes worker discovery and physical probing.
    ///
    /// Actual nonclone custody and every owning service move into a detached
    /// completion task before worker recording or actor commit awaits. Current
    /// request authority is checked afresh by each actor operation and recovery
    /// reread; no historical authenticated identity is reconstructed.
    async fn verify_retained_volume(
        &self,
        identity: &AuthenticatedIdentity,
        claim: &EffectClaim,
    ) -> Result<ResourceProgress, RetainedVolumeVerificationError> {
        Box::pin(execution::verify(self, identity, claim)).await
    }
}
