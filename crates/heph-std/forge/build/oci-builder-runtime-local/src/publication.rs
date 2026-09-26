//! Credentialed publication composition over verified local output.

use super::{
    config::{ForgeZotPublicationConfig, LocalOciRuntime},
    operation::VmOciOperation,
    output::verified_vm_publication_material,
};
use async_trait::async_trait;
use builder_catalog_domain::{OciDigest, OciImageReference};
use oci_builder_worker::{
    IsolatedOciBuild, OciBuildEngine, OciImageProductionOutput, OciOutputPublisher, OciWorkerError,
    RegistryPublisherTokenIssuer, RepositoryOciImagePublicationLease,
    RepositoryOciImagePublicationStore,
};
use registry_domain::{
    OciDescriptor, PublicationIntent, PublicationState, SupplyChainReferrerKind,
};
use registry_publisher::{CommandRunner, ControlledOciPublisher, PublicationMaterial};
use std::path::PathBuf;

/// Local OCI artifacts ready for the forge-controlled Zot publication step.
/// Verified files ready for the credentialed host publication boundary.
pub struct PreparedPublicationMaterial {
    /// Canonical descriptor read from the sealed OCI layout.
    pub expected_manifest: OciDescriptor,
    /// Image layout and bounded verifier evidence for publication.
    pub material: PublicationMaterial,
    /// Layout retained until rootfs materialization has completed.
    pub layout: PathBuf,
}

/// Combines local output processing with the forge's durable Zot publication protocol.
///
/// Buildah receives no registry token: the token is
/// acquired only after the local OCI index, scan, SBOM, and provenance exist.
pub struct ForgeZotOciPublisher<S, T, R> {
    runtime: LocalOciRuntime,
    legacy_tooling: Option<ForgeZotPublicationConfig>,
    publications: S,
    tokens: T,
    publisher: ControlledOciPublisher<R>,
}

/// Complete repository-image engine using separate builder and verifier VMs.
///
/// The embedded publisher is the only credentialed component. The VMs operate
/// entirely before it, with neither token nor registry client mounted.
pub struct VmPublishedOciEngine<S, T, R> {
    operation: VmOciOperation,
    publisher: ForgeZotOciPublisher<S, T, R>,
}

impl<S, T, R> VmPublishedOciEngine<S, T, R> {
    /// Composes VM operations with the host-controlled publisher.
    #[must_use]
    pub const fn new(operation: VmOciOperation, publisher: ForgeZotOciPublisher<S, T, R>) -> Self {
        Self {
            operation,
            publisher,
        }
    }
}

#[async_trait]
impl<S, T, R> OciBuildEngine for VmPublishedOciEngine<S, T, R>
where
    S: RepositoryOciImagePublicationStore,
    T: RegistryPublisherTokenIssuer,
    R: CommandRunner + Send + Sync + 'static,
{
    async fn build(
        &self,
        request: IsolatedOciBuild,
    ) -> Result<OciImageProductionOutput, OciWorkerError> {
        let output = self.operation.execute(&request).await?;
        let prepared = verified_vm_publication_material(&request, &output)?;
        self.publisher.publish_verified(&request, prepared).await
    }
}

impl<S, T, R> ForgeZotOciPublisher<S, T, R> {
    /// Creates the repository-image Zot publication adapter.
    #[must_use]
    pub const fn new(
        runtime: LocalOciRuntime,
        legacy_tooling: Option<ForgeZotPublicationConfig>,
        publications: S,
        tokens: T,
        publisher: ControlledOciPublisher<R>,
    ) -> Self {
        Self {
            runtime,
            legacy_tooling,
            publications,
            tokens,
            publisher,
        }
    }

    /// Publishes already independently verified OCI material.
    ///
    /// # Errors
    ///
    /// Returns a worker error if token issuance, registry publication, or
    /// registry read-back cannot prove the exact verified descriptor.
    pub async fn publish_verified(
        &self,
        request: &IsolatedOciBuild,
        prepared: PreparedPublicationMaterial,
    ) -> Result<OciImageProductionOutput, OciWorkerError>
    where
        S: RepositoryOciImagePublicationStore,
        T: RegistryPublisherTokenIssuer,
        R: CommandRunner + Send + Sync + 'static,
    {
        let lease = self
            .publications
            .begin_repository_image_publication(
                request.project_id,
                request.image_id,
                prepared.expected_manifest.clone(),
            )
            .await?;
        let approved = match lease {
            RepositoryOciImagePublicationLease::Approved(intent) => intent,
            RepositoryOciImagePublicationLease::Publish(intent) => {
                let token = match self.tokens.issue_pull_push(&intent).await {
                    Ok(token) => token,
                    Err(error) => {
                        self.publications
                            .retry_repository_image_publication(intent.id())
                            .await?;
                        return Err(error);
                    }
                };
                // Skopeo and ORAS are synchronous trusted host tools. The
                // token is issued only here, after the separate verifier VM
                // has completed, and is never visible to either VM.
                let verified = tokio::task::block_in_place(|| {
                    self.publisher
                        .publish(&intent, &prepared.material, token.token())
                })
                .map_err(|_| OciWorkerError::RegistryPublication);
                let verification = match verified {
                    Ok(verification) => verification,
                    Err(error) => {
                        self.publications
                            .retry_repository_image_publication(intent.id())
                            .await?;
                        return Err(error);
                    }
                };
                self.publications
                    .record_verified_and_approve(intent.id(), verification)
                    .await?
            }
        };
        zot_confirmed_output(&approved, prepared.layout)
    }
}

#[async_trait]
impl<S, T, R> OciOutputPublisher for ForgeZotOciPublisher<S, T, R>
where
    S: RepositoryOciImagePublicationStore,
    T: RegistryPublisherTokenIssuer,
    R: CommandRunner + Send + Sync + 'static,
{
    async fn publish(
        &self,
        request: &IsolatedOciBuild,
    ) -> Result<OciImageProductionOutput, OciWorkerError> {
        let prepared = self
            .runtime
            .publication_material(
                request,
                self.legacy_tooling
                    .as_ref()
                    .ok_or(OciWorkerError::InvalidConfiguration)?,
            )
            .await?;
        self.publish_verified(request, prepared).await
    }
}

pub fn zot_confirmed_output(
    intent: &PublicationIntent,
    local_oci_layout: PathBuf,
) -> Result<OciImageProductionOutput, OciWorkerError> {
    if intent.state() != PublicationState::Approved {
        return Err(OciWorkerError::RegistryPublication);
    }
    let reference = intent
        .approved_reference()
        .map_err(|_| OciWorkerError::RegistryPublication)?;
    let verification = intent
        .verification()
        .ok_or(OciWorkerError::RegistryPublication)?;
    let evidence_reference = |kind| -> Result<String, OciWorkerError> {
        let referrer = verification
            .evidence()
            .referrers()
            .iter()
            .find(|referrer| referrer.kind() == kind)
            .ok_or(OciWorkerError::RegistryPublication)?;
        Ok(format!(
            "{}/{}@{}",
            reference.authority(),
            reference.namespace(),
            referrer.descriptor().digest()
        ))
    };
    let image_reference = OciImageReference::parse(reference.to_string())
        .map_err(|_| OciWorkerError::RegistryPublication)?;
    let image_digest = OciDigest::parse(reference.digest().as_str().to_owned())
        .map_err(|_| OciWorkerError::RegistryPublication)?;
    Ok(OciImageProductionOutput {
        image_reference,
        image_digest,
        attestation_reference: evidence_reference(SupplyChainReferrerKind::Provenance)?,
        sbom_reference: Some(evidence_reference(SupplyChainReferrerKind::Sbom)?),
        scan_reference: evidence_reference(SupplyChainReferrerKind::Scan)?,
        local_oci_layout,
    })
}
