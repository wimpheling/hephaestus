use registry_domain::{
    ImmutableManifestReference, OciDescriptor, PublicationIntent, PublicationState,
    VerifiedPublication,
};
use registry_token::BearerToken;
use std::path::Path;

use super::{
    config::{CommandRunner, CommandSpec, PublicationMaterial, PublisherConfiguration},
    constants::SIGNATURE_ARTIFACT_TYPE,
    errors::PublisherError,
    layout::{EvidenceLayout, ValidatedEvidenceFile, ValidatedMaterial, validated_evidence},
    paths::{os_arguments, trusted_directory, validate_local_layout},
    read_client::{LazyHttpRegistryReadClient, RegistryReadClient},
};

/// Controlled publisher with replaceable command and registry-read adapters.
pub struct ControlledOciPublisher<R, C = LazyHttpRegistryReadClient> {
    pub(super) configuration: PublisherConfiguration,
    pub(super) runner: R,
    pub(super) reader: C,
}

impl<R> ControlledOciPublisher<R>
where
    R: CommandRunner,
{
    /// Creates a controlled publisher from validated administrator configuration.
    #[must_use]
    pub fn new(configuration: PublisherConfiguration, runner: R) -> Self {
        let reader = LazyHttpRegistryReadClient::new(configuration.registry_origin.clone());
        Self {
            configuration,
            runner,
            reader,
        }
    }
}

impl<R, C> ControlledOciPublisher<R, C>
where
    R: CommandRunner,
    C: RegistryReadClient,
{
    /// Creates a controlled publisher with an injected deterministic reader.
    #[must_use]
    pub const fn with_read_client(
        configuration: PublisherConfiguration,
        runner: R,
        reader: C,
    ) -> Self {
        Self {
            configuration,
            runner,
            reader,
        }
    }

    /// Copies, attaches evidence, reads Zot back, and returns verified evidence.
    ///
    /// `intent` is not mutated here: the `PostgreSQL` lifecycle adapter must
    /// atomically record the returned value before moving it to approval.
    ///
    /// # Errors
    ///
    /// Returns a typed error for unsafe local inputs, a failed OCI client
    /// command, or any disagreement between the immutable intent and Zot.
    pub fn publish(
        &self,
        intent: &PublicationIntent,
        material: &PublicationMaterial,
        token: &BearerToken,
    ) -> Result<VerifiedPublication, PublisherError> {
        self.assert_intent(intent)?;
        let local = self.validate_local_material(intent, material)?;
        self.copy_layout(intent.reference(), &local.layout, &local.source_tag, token)?;
        for evidence in &local.evidence {
            self.publish_evidence_layout(intent.reference(), &local.subject, evidence, token)?;
        }
        let signature_present = local
            .evidence
            .iter()
            .any(|evidence| evidence.artifact_type == SIGNATURE_ARTIFACT_TYPE);
        self.verify_remote(intent, token, signature_present)
    }

    /// Re-verifies a publication marked missing without republishing it.
    ///
    /// The lifecycle store only restores `missing` content after the exact
    /// immutable layout and its remote evidence graph have been checked again.
    ///
    /// # Errors
    ///
    /// Returns an error when the intent is not retryable or the exact
    /// immutable publication cannot be verified.
    pub fn verify_existing(
        &self,
        intent: &PublicationIntent,
        material: &PublicationMaterial,
        token: &BearerToken,
    ) -> Result<VerifiedPublication, PublisherError> {
        if intent.state() != PublicationState::Missing {
            return Err(PublisherError::NonRetryableIntent);
        }
        let local = self.validate_local_material(intent, material)?;
        let signature_present = local
            .evidence
            .iter()
            .any(|evidence| evidence.artifact_type == SIGNATURE_ARTIFACT_TYPE);
        self.verify_remote(intent, token, signature_present)
    }

    fn assert_intent(&self, intent: &PublicationIntent) -> Result<(), PublisherError> {
        if intent.reference().authority() != &self.configuration.authority {
            return Err(PublisherError::AuthorityMismatch);
        }
        match intent.state() {
            PublicationState::Pending | PublicationState::Publishing => Ok(()),
            PublicationState::Verified | PublicationState::Approved => {
                Err(PublisherError::AlreadyVerified)
            }
            PublicationState::Retired | PublicationState::Missing => {
                Err(PublisherError::NonRetryableIntent)
            }
        }
    }

    fn validate_local_material(
        &self,
        intent: &PublicationIntent,
        material: &PublicationMaterial,
    ) -> Result<ValidatedMaterial, PublisherError> {
        let layout = trusted_directory(&self.configuration.layout_root, &material.layout)?;
        let source_tag = validate_local_layout(&layout, intent.expected_manifest())?;
        let policy = intent.supply_chain_policy();
        let evidence = validated_evidence(
            &self.configuration.evidence_root,
            &material.evidence,
            policy,
        )?;
        Ok(ValidatedMaterial {
            layout,
            source_tag,
            subject: intent.expected_manifest().clone(),
            evidence,
        })
    }

    fn copy_layout(
        &self,
        reference: &ImmutableManifestReference,
        layout: &Path,
        source_tag: &str,
        token: &BearerToken,
    ) -> Result<(), PublisherError> {
        let source = format!("oci:{}:{source_tag}", layout.display());
        // OCI clients publish a manifest through a tag-like reference. The
        // tag is deterministic from the expected digest; all subsequent reads
        // and every product record use the immutable digest reference.
        let destination = format!(
            "docker://{}/{}:{source_tag}",
            reference.authority(),
            reference.namespace()
        );
        let mut arguments = os_arguments([
            "copy".into(),
            "--all".into(),
            "--preserve-digests".into(),
            // Never make publication depend on an ambient auth.json. The
            // destination bearer token below is the sole credential allowed
            // by the controlled publisher boundary.
            "--dest-no-creds".into(),
            "--dest-registry-token".into(),
            token.as_str().into(),
        ]);
        // The ordinary configured registry origin is HTTPS. A development
        // operator may explicitly supply a loopback HTTP Zot origin; only in
        // that opt-in case may Skopeo use cleartext for the destination.
        if self.configuration.registry_origin.scheme() == "http" {
            arguments.push("--dest-tls-verify=false".into());
        }
        arguments.extend([source.into(), destination.into()]);
        self.run(
            &CommandSpec::new(self.configuration.skopeo_binary.clone(), arguments)
                // Skopeo requires the literal bearer token here. It must never
                // appear in Debug output.
                .with_sensitive_argument(5),
        )
        .map(|_| ())
    }

    fn publish_evidence_layout(
        &self,
        reference: &ImmutableManifestReference,
        subject: &OciDescriptor,
        evidence: &ValidatedEvidenceFile,
        token: &BearerToken,
    ) -> Result<(), PublisherError> {
        let layout = EvidenceLayout::create(
            &self.configuration.credential_root,
            reference,
            subject,
            evidence,
        )?;
        self.copy_layout(reference, layout.path(), layout.tag(), token)
    }
}
