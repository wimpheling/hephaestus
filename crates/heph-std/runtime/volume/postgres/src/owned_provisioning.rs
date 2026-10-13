//! Canonical sealed backing purpose, actor admission and worker observation.

mod admission;
mod discovery;
mod first_ready;
mod observations;
pub mod storage;

use async_trait::async_trait;
use identity_domain::AuthenticatedIdentity;
use volume_trait::{
    BeginOwnedProvisioning, OwnedBackingObservation, OwnedProvisioningClaim,
    OwnedProvisioningContext, OwnedProvisioningDiscovery, OwnedProvisioningExpectation,
    VolumeError, VolumeOwnedProvisioningRepository,
};

use crate::PostgresVolumeMetadataRepository;

#[async_trait]
impl VolumeOwnedProvisioningRepository for PostgresVolumeMetadataRepository {
    async fn begin_owned_provisioning(
        &self,
        identity: &AuthenticatedIdentity,
        request: &BeginOwnedProvisioning,
    ) -> Result<OwnedProvisioningContext, VolumeError> {
        admission::begin(self, identity, request).await
    }
    async fn discover_owned_provisioning(
        &self,
        expected: &OwnedProvisioningExpectation,
    ) -> Result<OwnedProvisioningDiscovery, VolumeError> {
        discovery::inspect(self, expected).await
    }
    async fn assert_owned_first_ready_verification(
        &self,
        claim: &OwnedProvisioningClaim,
    ) -> Result<(), VolumeError> {
        first_ready::assert_unfinished(self, claim).await
    }
    async fn owned_reconciliation_context(
        &self,
        claim: &OwnedProvisioningClaim,
    ) -> Result<OwnedProvisioningContext, VolumeError> {
        observations::inspect(self, claim).await
    }
    async fn record_owned_observation(
        &self,
        claim: &OwnedProvisioningClaim,
        observation: &OwnedBackingObservation,
    ) -> Result<OwnedProvisioningContext, VolumeError> {
        observations::record(self, claim, observation).await
    }
}
