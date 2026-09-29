use registry_domain::{
    ImmutableManifestReference, OciDescriptor, OciMediaType, PublicationIntent, Sha256Digest,
    SupplyChainEvidence, SupplyChainPolicy, SupplyChainReferrer, VerifiedPublication,
};
use registry_token::BearerToken;

use super::{
    config::{CommandOutput, CommandRunner, CommandSpec},
    constants::OCI_MANIFEST_MEDIA_TYPE,
    errors::PublisherError,
    paths::is_authentication_failure,
    publisher::ControlledOciPublisher,
    read_client::RegistryReadClient,
    remote::{
        RemoteManifest, RemoteReferrerDescriptor, RemoteReferrers, artifact_type, parse_platforms,
        required_kinds, validate_manifest_bytes,
    },
};

impl<R, C> ControlledOciPublisher<R, C>
where
    R: CommandRunner,
    C: RegistryReadClient,
{
    pub(super) fn verify_remote(
        &self,
        intent: &PublicationIntent,
        token: &BearerToken,
        signature_present: bool,
    ) -> Result<VerifiedPublication, PublisherError> {
        let reference = intent.reference();
        let manifest =
            self.remote_descriptor(reference.namespace().as_str(), reference.digest(), token)?;
        if manifest != *intent.expected_manifest() {
            return Err(PublisherError::WrongRemoteDescriptor);
        }
        let raw_manifest = self.remote_manifest(
            reference.namespace().as_str(),
            manifest.digest(),
            &manifest,
            token,
        )?;
        let platforms = parse_platforms(&raw_manifest, &manifest)?;
        let discovered = self.remote_referrers(reference, token)?;
        let evidence = self.verify_referrers(
            reference,
            &discovered,
            token,
            intent.supply_chain_policy(),
            signature_present,
        )?;
        VerifiedPublication::new(reference, manifest, platforms, evidence)
            .map_err(PublisherError::Domain)
    }

    fn remote_descriptor(
        &self,
        namespace: &str,
        digest: &Sha256Digest,
        token: &BearerToken,
    ) -> Result<OciDescriptor, PublisherError> {
        self.reader
            .manifest_descriptor(namespace, digest, token)
            .map_err(PublisherError::RegistryRead)
    }

    fn remote_manifest(
        &self,
        namespace: &str,
        digest: &Sha256Digest,
        descriptor: &OciDescriptor,
        token: &BearerToken,
    ) -> Result<RemoteManifest, PublisherError> {
        let bytes = self
            .reader
            .manifest_bytes(namespace, digest, token)
            .map_err(PublisherError::RegistryRead)?;
        validate_manifest_bytes(&bytes, descriptor)?;
        serde_json::from_slice(&bytes).map_err(|_| PublisherError::MalformedRemoteManifest)
    }

    fn remote_referrers(
        &self,
        reference: &ImmutableManifestReference,
        token: &BearerToken,
    ) -> Result<Vec<RemoteReferrerDescriptor>, PublisherError> {
        let bytes = self
            .reader
            .referrers_bytes(reference.namespace().as_str(), reference.digest(), token)
            .map_err(PublisherError::RegistryRead)?;
        let discovery: RemoteReferrers =
            serde_json::from_slice(&bytes).map_err(|_| PublisherError::MalformedReferrers)?;
        Ok(discovery.manifests)
    }

    fn verify_referrers(
        &self,
        reference: &ImmutableManifestReference,
        discovered: &[RemoteReferrerDescriptor],
        token: &BearerToken,
        policy: SupplyChainPolicy,
        signature_present: bool,
    ) -> Result<SupplyChainEvidence, PublisherError> {
        let kinds = required_kinds(policy, signature_present);
        let mut verified = Vec::with_capacity(kinds.len());
        for kind in kinds {
            let expected_type = artifact_type(kind);
            let matching = discovered
                .iter()
                .filter(|descriptor| descriptor.artifact_type.as_deref() == Some(expected_type))
                .collect::<Vec<_>>();
            let [descriptor] = matching.as_slice() else {
                return Err(PublisherError::MissingOrDuplicateReferrer(kind));
            };
            let descriptor = descriptor.to_domain()?;
            let remote_descriptor =
                self.remote_descriptor(reference.namespace().as_str(), descriptor.digest(), token)?;
            if remote_descriptor != descriptor {
                return Err(PublisherError::WrongReferrerDescriptor);
            }
            let manifest = self.remote_manifest(
                reference.namespace().as_str(),
                remote_descriptor.digest(),
                &remote_descriptor,
                token,
            )?;
            if manifest.media_type.as_deref() != Some(OCI_MANIFEST_MEDIA_TYPE)
                || manifest.artifact_type.as_deref() != Some(expected_type)
            {
                return Err(PublisherError::WrongReferrerDescriptor);
            }
            let subject = manifest
                .subject
                .as_ref()
                .map(|subject| subject.digest.as_str())
                .ok_or(PublisherError::WrongReferrerSubject)?;
            let subject =
                Sha256Digest::parse(subject.to_owned()).map_err(PublisherError::Domain)?;
            if subject != *reference.digest() {
                return Err(PublisherError::WrongReferrerSubject);
            }
            verified.push(SupplyChainReferrer::new(
                kind,
                subject,
                descriptor,
                OciMediaType::parse(expected_type.to_owned()).map_err(PublisherError::Domain)?,
            ));
        }
        SupplyChainEvidence::new(reference.digest().clone(), verified)
            .map_err(PublisherError::Domain)
    }

    pub(super) fn run(&self, command: &CommandSpec) -> Result<CommandOutput, PublisherError> {
        let output = self.runner.run(command).map_err(PublisherError::Runner)?;
        if output.success {
            Ok(output)
        } else if is_authentication_failure(&output.stderr) {
            Err(PublisherError::AuthenticationFailed)
        } else {
            Err(PublisherError::CommandFailed)
        }
    }
}
