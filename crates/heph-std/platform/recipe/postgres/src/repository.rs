use crate::PostgresDeploymentRepository;
use async_trait::async_trait;
use identity_domain::AuthenticatedIdentity;
use recipe_application::{
    AttemptProvenance, CommandIdentity, CommandReceipt, DeploymentAdmission, DeploymentError,
    DeploymentId, DeploymentRepository, DeploymentSnapshot, DiagnosticCode, EffectClaim,
    EffectEvidence, InstallDeployment, ReconciledOutcome, RemoveDeployment, ResourceEffect,
    ResourceProgress,
};

#[async_trait]
impl DeploymentRepository for PostgresDeploymentRepository {
    async fn admit_install(
        &self,
        identity: &AuthenticatedIdentity,
        command: InstallDeployment,
    ) -> Result<DeploymentAdmission, DeploymentError> {
        Self::admit_install(self, identity, command).await
    }
    async fn inspect(
        &self,
        identity: &AuthenticatedIdentity,
        id: DeploymentId,
    ) -> Result<DeploymentSnapshot, DeploymentError> {
        Self::inspect(self, identity, id).await
    }
    async fn admit_remove(
        &self,
        identity: &AuthenticatedIdentity,
        command: RemoveDeployment,
    ) -> Result<DeploymentAdmission, DeploymentError> {
        Self::admit_remove(self, identity, command).await
    }
    async fn claim_resource_effect(
        &self,
        identity: &AuthenticatedIdentity,
        effect: ResourceEffect,
    ) -> Result<EffectClaim, DeploymentError> {
        Self::claim_resource_effect(self, identity, effect).await
    }
    async fn complete_resource_effect(
        &self,
        identity: &AuthenticatedIdentity,
        claim: &EffectClaim,
        evidence: EffectEvidence,
    ) -> Result<ResourceProgress, DeploymentError> {
        Self::complete_resource_effect(self, identity, claim, evidence).await
    }
    async fn mark_resource_ambiguous(
        &self,
        identity: &AuthenticatedIdentity,
        claim: &EffectClaim,
        diagnostic: DiagnosticCode,
    ) -> Result<ResourceProgress, DeploymentError> {
        Self::mark_resource_ambiguous(self, identity, claim, diagnostic).await
    }
    async fn record_reconciliation(
        &self,
        identity: &AuthenticatedIdentity,
        command: CommandIdentity,
        claim: &EffectClaim,
        provenance: AttemptProvenance,
        outcome: ReconciledOutcome,
    ) -> Result<ResourceProgress, DeploymentError> {
        Self::record_reconciliation(self, identity, command, claim, provenance, outcome).await
    }
    async fn finish_command(
        &self,
        identity: &AuthenticatedIdentity,
        command: CommandIdentity,
        id: DeploymentId,
        version: u64,
    ) -> Result<CommandReceipt, DeploymentError> {
        Self::finish_command(self, identity, command, id, version).await
    }
}
