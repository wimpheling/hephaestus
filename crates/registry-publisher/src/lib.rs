//! Controlled publication of administrator-owned OCI layouts to Zot.
//!
//! This adapter never accepts a caller-selected registry, repository, command,
//! or credential.  The durable [`PublicationIntent`] defines the sole remote
//! subject; the configured registry authority and local roots form the other
//! half of that trust boundary.

use registry_domain::{
    ImmutableManifestReference, OciDescriptor, OciMediaType, PlatformDescriptor, PublicationIntent,
    PublicationState, RegistryAuthority, RegistryValueError, Sha256Digest, SupplyChainEvidence,
    SupplyChainPolicy, SupplyChainReferrer, SupplyChainReferrerKind, VerifiedPublication,
};
use registry_token::BearerToken;
use reqwest::{
    StatusCode, Url,
    blocking::{Client as HttpClient, Response as HttpResponse},
    header,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    ffi::OsString,
    fmt,
    fs::{self, File},
    io::{BufReader, Read},
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};
use tempfile::TempDir;

const OCI_IMAGE_LAYOUT_VERSION: &str = "1.0.0";
const OCI_INDEX_MEDIA_TYPE: &str = "application/vnd.oci.image.index.v1+json";
const OCI_MANIFEST_MEDIA_TYPE: &str = "application/vnd.oci.image.manifest.v1+json";
const SBOM_ARTIFACT_TYPE: &str = "application/spdx+json";
const PROVENANCE_ARTIFACT_TYPE: &str = "application/vnd.in-toto+json";
const SCAN_ARTIFACT_TYPE: &str = "application/vnd.hephaestus.vulnerability-scan.v1+json";
const SIGNATURE_ARTIFACT_TYPE: &str = "application/vnd.dev.cosign.simplesigning.v1+json";
const OCI_REFERENCE_NAME_ANNOTATION: &str = "org.opencontainers.image.ref.name";
const MAX_REGISTRY_DOCUMENT_BYTES: usize = 4 * 1024 * 1024;
const REGISTRY_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Immutable, administrator-owned locations and binaries for OCI publication.
#[derive(Debug, Clone)]
pub struct PublisherConfiguration {
    authority: RegistryAuthority,
    layout_root: PathBuf,
    credential_root: PathBuf,
    skopeo_binary: PathBuf,
    registry_origin: Url,
    registry_client: HttpClient,
}

impl PublisherConfiguration {
    /// Validates the administrator-controlled publisher boundary.
    ///
    /// # Errors
    ///
    /// Returns an error when a root or executable is relative, missing, or a
    /// symbolic link. The publication inputs must be descendants of
    /// `layout_root`; temporary credential files are created below
    /// `credential_root` and removed before this adapter returns.
    pub fn new(
        authority: RegistryAuthority,
        layout_root: &Path,
        credential_root: &Path,
        skopeo_binary: &Path,
        _oras_binary: &Path,
    ) -> Result<Self, PublisherError> {
        let registry_client = HttpClient::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(REGISTRY_REQUEST_TIMEOUT)
            .build()
            .map_err(|_| PublisherError::RegistryRead(RegistryReadError::Failed))?;
        let registry_origin = registry_origin(&authority)?;
        Ok(Self {
            authority,
            layout_root: canonical_directory(layout_root)?,
            credential_root: canonical_directory(credential_root)?,
            skopeo_binary: canonical_executable(skopeo_binary)?,
            registry_origin,
            registry_client,
        })
    }

    /// Returns the configured, fixed registry authority.
    #[must_use]
    pub const fn authority(&self) -> &RegistryAuthority {
        &self.authority
    }

    /// Replaces the registry read origin for an explicitly configured private
    /// endpoint. This supports local loopback Zot without weakening the
    /// default HTTPS origin derived from the public authority.
    ///
    /// # Errors
    ///
    /// Returns an error unless `origin` is a credential-free HTTP(S) origin.
    pub fn with_registry_origin(mut self, origin: &str) -> Result<Self, PublisherError> {
        self.registry_origin = parse_registry_origin(origin)?;
        Ok(self)
    }
}

/// Evidence files produced by the trusted build and scanning stages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicationEvidenceFiles {
    /// SPDX JSON software bill of materials.
    pub sbom: PathBuf,
    /// In-toto build provenance statement.
    pub provenance: PathBuf,
    /// Trusted vulnerability scan result.
    pub scan: PathBuf,
    /// Optional signing or approval artifact.
    pub signature: Option<PathBuf>,
}

/// The local, administrator-owned publication material for one durable intent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicationMaterial {
    /// OCI image layout directory, below the configured layout root.
    pub layout: PathBuf,
    /// Required and optional supply-chain evidence files.
    pub evidence: PublicationEvidenceFiles,
}

/// A command to execute without a shell.
#[derive(Clone, PartialEq, Eq)]
pub struct CommandSpec {
    program: PathBuf,
    arguments: Vec<OsString>,
    environment: Vec<(OsString, OsString)>,
    sensitive_argument_positions: Vec<usize>,
}

impl CommandSpec {
    const fn new(program: PathBuf, arguments: Vec<OsString>) -> Self {
        Self {
            program,
            arguments,
            environment: Vec::new(),
            sensitive_argument_positions: Vec::new(),
        }
    }

    fn with_sensitive_argument(mut self, position: usize) -> Self {
        self.sensitive_argument_positions.push(position);
        self
    }

    /// Returns the executable path.
    #[must_use]
    pub fn program(&self) -> &Path {
        &self.program
    }

    /// Returns literal command arguments.
    #[must_use]
    pub fn arguments(&self) -> &[OsString] {
        &self.arguments
    }

    /// Returns the narrowly scoped subprocess environment.
    #[must_use]
    pub fn environment_entries(&self) -> &[(OsString, OsString)] {
        &self.environment
    }
}

impl fmt::Debug for CommandSpec {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let arguments = self
            .arguments
            .iter()
            .enumerate()
            .map(|(position, argument)| {
                if self.sensitive_argument_positions.contains(&position) {
                    String::from("REDACTED")
                } else {
                    argument.to_string_lossy().into_owned()
                }
            })
            .collect::<Vec<_>>();
        formatter
            .debug_struct("CommandSpec")
            .field("program", &self.program)
            .field("arguments", &arguments)
            .field("environment", &"REDACTED")
            .finish()
    }
}

/// Captured result from an injectable command runner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    /// Process exit status as supplied by the runner.
    pub success: bool,
    /// Standard output bytes.
    pub stdout: Vec<u8>,
    /// Standard error bytes, retained only until classified as a failure.
    pub stderr: Vec<u8>,
}

impl CommandOutput {
    /// Creates a successful captured command result.
    #[must_use]
    pub fn success(stdout: impl Into<Vec<u8>>) -> Self {
        Self {
            success: true,
            stdout: stdout.into(),
            stderr: Vec::new(),
        }
    }

    /// Creates an unsuccessful captured command result.
    #[must_use]
    pub fn failure(stderr: impl Into<Vec<u8>>) -> Self {
        Self {
            success: false,
            stdout: Vec::new(),
            stderr: stderr.into(),
        }
    }
}

/// Executes one literal publisher command.
pub trait CommandRunner: Send + Sync {
    /// Executes `command` without inheriting an ambient shell.
    ///
    /// # Errors
    ///
    /// Returns only a non-sensitive process-launch failure.
    fn run(&self, command: &CommandSpec) -> Result<CommandOutput, CommandRunnerError>;
}

/// Reads the immutable OCI Distribution data required to verify a publication.
///
/// Implementations receive only an already scoped bearer token and must not
/// follow redirects or expose that token in errors.
pub trait RegistryReadClient: Send + Sync {
    /// Reads the descriptor returned for an exact manifest digest.
    ///
    /// # Errors
    ///
    /// Returns an opaque error when the registry rejects the scoped token or
    /// cannot return a bounded valid response.
    fn manifest_descriptor(
        &self,
        namespace: &str,
        digest: &Sha256Digest,
        token: &BearerToken,
    ) -> Result<OciDescriptor, RegistryReadError>;

    /// Reads the raw bytes for an exact manifest digest.
    ///
    /// # Errors
    ///
    /// Returns an opaque error when the registry rejects the scoped token or
    /// cannot return a bounded valid response.
    fn manifest_bytes(
        &self,
        namespace: &str,
        digest: &Sha256Digest,
        token: &BearerToken,
    ) -> Result<Vec<u8>, RegistryReadError>;

    /// Reads the raw OCI referrers index for an exact subject digest.
    ///
    /// # Errors
    ///
    /// Returns an opaque error when the registry rejects the scoped token or
    /// cannot return a bounded valid response.
    fn referrers_bytes(
        &self,
        namespace: &str,
        subject: &Sha256Digest,
        token: &BearerToken,
    ) -> Result<Vec<u8>, RegistryReadError>;
}

/// Opaque failure from a bounded OCI read operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RegistryReadError {
    /// The scoped bearer token was rejected by the registry.
    #[error("registry authentication failed")]
    AuthenticationFailed,
    /// The registry could not provide a valid bounded response.
    #[error("registry read failed")]
    Failed,
}

/// The real process runner used by the trusted publisher service.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemCommandRunner;

// `env_clear` is intentional for publisher subprocess isolation, but OCI
// tools may invoke standard helper binaries such as `newuidmap`.
const TRUSTED_SYSTEM_PATH: &str = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";

impl CommandRunner for SystemCommandRunner {
    fn run(&self, command: &CommandSpec) -> Result<CommandOutput, CommandRunnerError> {
        let output = Command::new(&command.program)
            .env_clear()
            .env("PATH", TRUSTED_SYSTEM_PATH)
            .envs(command.environment.iter().map(|(key, value)| (key, value)))
            .stdin(Stdio::null())
            .args(&command.arguments)
            .output()
            .map_err(|_| CommandRunnerError::LaunchFailed)?;
        if !output.status.success() {
            // The command specification redacts its bearer-token argument;
            // subprocess diagnostics contain the actionable registry error
            // without reproducing that secret. Keep them bounded so a failed
            // OCI client cannot flood the trusted release operator's logs.
            let stderr = String::from_utf8_lossy(&output.stderr);
            eprintln!(
                "trusted OCI command {} failed ({}): {}",
                command.program.display(),
                output.status,
                stderr.chars().take(4_096).collect::<String>().trim()
            );
        }
        Ok(CommandOutput {
            success: output.status.success(),
            stdout: output.stdout,
            stderr: output.stderr,
        })
    }
}

/// Redirect-free HTTP implementation of [`RegistryReadClient`].
pub struct HttpRegistryReadClient {
    origin: Url,
    client: HttpClient,
}

impl HttpRegistryReadClient {
    const fn new(origin: Url, client: HttpClient) -> Self {
        Self { origin, client }
    }

    fn request(
        &self,
        namespace: &str,
        suffix: &str,
        token: &BearerToken,
        head: bool,
    ) -> Result<HttpResponse, RegistryReadError> {
        let url = self
            .origin
            .join(&format!("v2/{namespace}/{suffix}"))
            .map_err(|_| RegistryReadError::Failed)?;
        let request = if head {
            self.client.head(url)
        } else {
            self.client.get(url)
        };
        let response = request
            .bearer_auth(token.as_str())
            .header(
                header::ACCEPT,
                "application/vnd.oci.image.index.v1+json, application/vnd.oci.image.manifest.v1+json, application/vnd.oci.image.referrers.v1+json",
            )
            .send()
            .map_err(|_| RegistryReadError::Failed)?;
        if response.status() == StatusCode::UNAUTHORIZED
            || response.status() == StatusCode::FORBIDDEN
        {
            return Err(RegistryReadError::AuthenticationFailed);
        }
        response
            .status()
            .is_success()
            .then_some(response)
            .ok_or(RegistryReadError::Failed)
    }
}

impl RegistryReadClient for HttpRegistryReadClient {
    fn manifest_descriptor(
        &self,
        namespace: &str,
        digest: &Sha256Digest,
        token: &BearerToken,
    ) -> Result<OciDescriptor, RegistryReadError> {
        let response = self.request(namespace, &format!("manifests/{digest}"), token, true)?;
        let media_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(';').next())
            .ok_or(RegistryReadError::Failed)?;
        let digest = response
            .headers()
            .get("docker-content-digest")
            .and_then(|value| value.to_str().ok())
            .ok_or(RegistryReadError::Failed)?;
        let size = response
            .headers()
            .get(header::CONTENT_LENGTH)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok())
            .ok_or(RegistryReadError::Failed)?;
        OciDescriptor::new(
            Sha256Digest::parse(digest.to_owned()).map_err(|_| RegistryReadError::Failed)?,
            size,
            OciMediaType::parse(media_type.to_owned()).map_err(|_| RegistryReadError::Failed)?,
        )
        .map_err(|_| RegistryReadError::Failed)
    }

    fn manifest_bytes(
        &self,
        namespace: &str,
        digest: &Sha256Digest,
        token: &BearerToken,
    ) -> Result<Vec<u8>, RegistryReadError> {
        bounded_response(self.request(namespace, &format!("manifests/{digest}"), token, false)?)
    }

    fn referrers_bytes(
        &self,
        namespace: &str,
        subject: &Sha256Digest,
        token: &BearerToken,
    ) -> Result<Vec<u8>, RegistryReadError> {
        bounded_response(self.request(namespace, &format!("referrers/{subject}"), token, false)?)
    }
}

/// Controlled publisher with replaceable command and registry-read adapters.
pub struct ControlledOciPublisher<R, C = HttpRegistryReadClient> {
    configuration: PublisherConfiguration,
    runner: R,
    reader: C,
}

impl<R> ControlledOciPublisher<R>
where
    R: CommandRunner,
{
    /// Creates a controlled publisher from validated administrator configuration.
    #[must_use]
    pub fn new(configuration: PublisherConfiguration, runner: R) -> Self {
        let reader = HttpRegistryReadClient::new(
            configuration.registry_origin.clone(),
            configuration.registry_client.clone(),
        );
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
        let evidence =
            validated_evidence(&self.configuration.layout_root, &material.evidence, policy)?;
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
                .with_sensitive_argument(4),
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

    fn verify_remote(
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

    fn run(&self, command: &CommandSpec) -> Result<CommandOutput, PublisherError> {
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

#[derive(Debug)]
struct ValidatedMaterial {
    layout: PathBuf,
    source_tag: String,
    subject: OciDescriptor,
    evidence: Vec<ValidatedEvidenceFile>,
}

#[derive(Debug)]
struct ValidatedEvidenceFile {
    path: PathBuf,
    artifact_type: &'static str,
}

/// A transient OCI layout containing one evidence artifact and its subject.
///
/// Skopeo publishes this standard layout with the same scoped bearer flow as
/// the image graph; the publisher never delegates bearer-token handling to an
/// artifact CLI.
struct EvidenceLayout {
    root: TempDir,
    tag: String,
}

impl EvidenceLayout {
    fn create(
        parent: &Path,
        _reference: &ImmutableManifestReference,
        subject: &OciDescriptor,
        evidence: &ValidatedEvidenceFile,
    ) -> Result<Self, PublisherError> {
        let root = TempDir::new_in(parent).map_err(PublisherError::Filesystem)?;
        let blobs = root.path().join("blobs/sha256");
        fs::create_dir_all(&blobs).map_err(PublisherError::Filesystem)?;
        fs::write(
            root.path().join("oci-layout"),
            r#"{"imageLayoutVersion":"1.0.0"}"#,
        )
        .map_err(PublisherError::Filesystem)?;
        let config = write_layout_blob(&blobs, b"{}")?;
        let evidence_bytes = fs::read(&evidence.path).map_err(PublisherError::Filesystem)?;
        let layer = write_layout_blob(&blobs, &evidence_bytes)?;
        let manifest = serde_json::json!({
            "schemaVersion": 2,
            "mediaType": OCI_MANIFEST_MEDIA_TYPE,
            "artifactType": evidence.artifact_type,
            "config": descriptor_json(&config, "application/vnd.oci.empty.v1+json"),
            "layers": [descriptor_json(&layer, evidence.artifact_type)],
            "subject": descriptor_json(subject, subject.media_type().as_str()),
        });
        let manifest_bytes =
            serde_json::to_vec(&manifest).map_err(|_| PublisherError::MalformedLocalLayout)?;
        let descriptor = write_layout_blob(&blobs, &manifest_bytes)?;
        let tag = local_reference_tag(descriptor.digest());
        let index = serde_json::json!({
            "schemaVersion": 2,
            "mediaType": OCI_INDEX_MEDIA_TYPE,
            "manifests": [{
                "mediaType": OCI_MANIFEST_MEDIA_TYPE,
                "digest": descriptor.digest().as_str(),
                "size": descriptor.size(),
                "annotations": { OCI_REFERENCE_NAME_ANNOTATION: tag },
            }],
        });
        fs::write(
            root.path().join("index.json"),
            serde_json::to_vec(&index).map_err(|_| PublisherError::MalformedLocalIndex)?,
        )
        .map_err(PublisherError::Filesystem)?;
        Ok(Self { root, tag })
    }

    fn path(&self) -> &Path {
        self.root.path()
    }
    fn tag(&self) -> &str {
        &self.tag
    }
}

fn write_layout_blob(blobs: &Path, bytes: &[u8]) -> Result<OciDescriptor, PublisherError> {
    let digest = Sha256Digest::parse(format!("sha256:{:x}", Sha256::digest(bytes)))
        .map_err(PublisherError::Domain)?;
    fs::write(blobs.join(&digest.as_str()["sha256:".len()..]), bytes)
        .map_err(PublisherError::Filesystem)?;
    OciDescriptor::new(
        digest,
        u64::try_from(bytes.len()).map_err(|_| PublisherError::MalformedLocalLayout)?,
        OciMediaType::parse(OCI_MANIFEST_MEDIA_TYPE.to_owned()).map_err(PublisherError::Domain)?,
    )
    .map_err(PublisherError::Domain)
}

fn descriptor_json(descriptor: &OciDescriptor, media_type: &str) -> serde_json::Value {
    serde_json::json!({
        "mediaType": media_type,
        "digest": descriptor.digest().as_str(),
        "size": descriptor.size(),
    })
}

fn validated_evidence(
    root: &Path,
    evidence: &PublicationEvidenceFiles,
    policy: SupplyChainPolicy,
) -> Result<Vec<ValidatedEvidenceFile>, PublisherError> {
    let mut files = vec![
        ValidatedEvidenceFile {
            path: trusted_file(root, &evidence.sbom)?,
            artifact_type: SBOM_ARTIFACT_TYPE,
        },
        ValidatedEvidenceFile {
            path: trusted_file(root, &evidence.provenance)?,
            artifact_type: PROVENANCE_ARTIFACT_TYPE,
        },
        ValidatedEvidenceFile {
            path: trusted_file(root, &evidence.scan)?,
            artifact_type: SCAN_ARTIFACT_TYPE,
        },
    ];
    match (&evidence.signature, policy.signature_required()) {
        (Some(path), _) => files.push(ValidatedEvidenceFile {
            path: trusted_file(root, path)?,
            artifact_type: SIGNATURE_ARTIFACT_TYPE,
        }),
        (None, true) => return Err(PublisherError::MissingSignature),
        (None, false) => {}
    }
    Ok(files)
}

const fn artifact_type(kind: SupplyChainReferrerKind) -> &'static str {
    match kind {
        SupplyChainReferrerKind::Sbom => SBOM_ARTIFACT_TYPE,
        SupplyChainReferrerKind::Provenance => PROVENANCE_ARTIFACT_TYPE,
        SupplyChainReferrerKind::Scan => SCAN_ARTIFACT_TYPE,
        SupplyChainReferrerKind::Signature => SIGNATURE_ARTIFACT_TYPE,
    }
}

fn required_kinds(
    policy: SupplyChainPolicy,
    signature_present: bool,
) -> Vec<SupplyChainReferrerKind> {
    let mut kinds = vec![
        SupplyChainReferrerKind::Sbom,
        SupplyChainReferrerKind::Provenance,
        SupplyChainReferrerKind::Scan,
    ];
    if policy.signature_required() || signature_present {
        kinds.push(SupplyChainReferrerKind::Signature);
    }
    kinds
}

fn os_arguments<const N: usize>(values: [OsString; N]) -> Vec<OsString> {
    values.into()
}

fn registry_origin(authority: &RegistryAuthority) -> Result<Url, PublisherError> {
    parse_registry_origin(&format!("https://{authority}"))
}

fn parse_registry_origin(value: &str) -> Result<Url, PublisherError> {
    let origin = Url::parse(value).map_err(|_| PublisherError::UnsafeRegistryOrigin)?;
    let valid = matches!(origin.scheme(), "http" | "https")
        && origin.host_str().is_some()
        && origin.username().is_empty()
        && origin.password().is_none()
        && origin.query().is_none()
        && origin.fragment().is_none()
        && origin.path() == "/";
    valid
        .then_some(origin)
        .ok_or(PublisherError::UnsafeRegistryOrigin)
}

fn bounded_response(response: HttpResponse) -> Result<Vec<u8>, RegistryReadError> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_REGISTRY_DOCUMENT_BYTES as u64)
    {
        return Err(RegistryReadError::Failed);
    }
    let mut bytes = Vec::new();
    let mut limited = response.take(u64::try_from(MAX_REGISTRY_DOCUMENT_BYTES + 1).expect("bound"));
    limited
        .read_to_end(&mut bytes)
        .map_err(|_| RegistryReadError::Failed)?;
    (!bytes.is_empty() && bytes.len() <= MAX_REGISTRY_DOCUMENT_BYTES)
        .then_some(bytes)
        .ok_or(RegistryReadError::Failed)
}

fn validate_manifest_bytes(bytes: &[u8], descriptor: &OciDescriptor) -> Result<(), PublisherError> {
    if u64::try_from(bytes.len()).map_err(|_| PublisherError::WrongRemoteManifestBytes)?
        != descriptor.size()
        || Sha256Digest::parse(format!("sha256:{:x}", Sha256::digest(bytes)))
            .map_err(PublisherError::Domain)?
            != *descriptor.digest()
    {
        return Err(PublisherError::WrongRemoteManifestBytes);
    }
    Ok(())
}

fn parse_platforms(
    manifest: &RemoteManifest,
    descriptor: &OciDescriptor,
) -> Result<Vec<PlatformDescriptor>, PublisherError> {
    if manifest.media_type.as_deref() != Some(OCI_INDEX_MEDIA_TYPE)
        || !descriptor.media_type().is_image_index()
        || manifest.manifests.is_empty()
    {
        return Err(PublisherError::MalformedRemoteIndex);
    }
    manifest
        .manifests
        .iter()
        .map(|entry| {
            let descriptor = entry.to_domain()?;
            let platform = entry
                .platform
                .as_ref()
                .ok_or(PublisherError::MalformedRemoteIndex)?;
            PlatformDescriptor::new(
                descriptor,
                platform.operating_system.clone(),
                platform.architecture.clone(),
                platform.variant.clone(),
            )
            .map_err(PublisherError::Domain)
        })
        .collect()
}

fn validate_local_layout(
    layout: &Path,
    expected: &OciDescriptor,
) -> Result<String, PublisherError> {
    let layout_file = safe_layout_file(layout, Path::new("oci-layout"))?;
    let layout_json: LocalLayoutVersion =
        serde_json::from_reader(File::open(layout_file).map_err(PublisherError::Filesystem)?)
            .map_err(|_| PublisherError::MalformedLocalLayout)?;
    if layout_json.image_layout_version != OCI_IMAGE_LAYOUT_VERSION {
        return Err(PublisherError::MalformedLocalLayout);
    }
    let index_file = safe_layout_file(layout, Path::new("index.json"))?;
    let index: LocalIndex =
        serde_json::from_reader(File::open(index_file).map_err(PublisherError::Filesystem)?)
            .map_err(|_| PublisherError::MalformedLocalIndex)?;
    let matching = index
        .manifests
        .iter()
        .filter(|descriptor| descriptor.digest == expected.digest().as_str())
        .collect::<Vec<_>>();
    let [local] = matching.as_slice() else {
        return Err(PublisherError::WrongLocalDigest);
    };
    if index.manifests.len() != 1 {
        return Err(PublisherError::AmbiguousLocalLayout);
    }
    if local.to_domain()? != *expected {
        return Err(PublisherError::WrongLocalDescriptor);
    }
    let digest_hex = expected
        .digest()
        .as_str()
        .strip_prefix("sha256:")
        .ok_or(PublisherError::WrongLocalDigest)?;
    let blob = safe_layout_file(
        layout,
        &PathBuf::from("blobs").join("sha256").join(digest_hex),
    )?;
    let metadata = fs::metadata(&blob).map_err(PublisherError::Filesystem)?;
    if metadata.len() != expected.size() || hash_file(&blob)? != *expected.digest() {
        return Err(PublisherError::WrongLocalDigest);
    }
    let subject_bytes = fs::read(&blob).map_err(PublisherError::Filesystem)?;
    let subject: RemoteManifest =
        serde_json::from_slice(&subject_bytes).map_err(|_| PublisherError::MalformedLocalIndex)?;
    parse_platforms(&subject, expected).map_err(|_| PublisherError::MalformedLocalIndex)?;
    let tag = local
        .annotations
        .get(OCI_REFERENCE_NAME_ANNOTATION)
        .filter(|value| value.as_str() == local_reference_tag(expected.digest()))
        .cloned()
        .ok_or(PublisherError::WrongLocalReferenceName)?;
    Ok(tag)
}

fn local_reference_tag(digest: &Sha256Digest) -> String {
    format!("heph-{}", digest.as_str().replace(':', "-"))
}

fn hash_file(path: &Path) -> Result<Sha256Digest, PublisherError> {
    let file = File::open(path).map_err(PublisherError::Filesystem)?;
    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 16_384];
    loop {
        let count = reader
            .read(&mut buffer)
            .map_err(PublisherError::Filesystem)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Sha256Digest::parse(format!("sha256:{:x}", hasher.finalize())).map_err(PublisherError::Domain)
}

fn canonical_directory(path: &Path) -> Result<PathBuf, PublisherError> {
    if !path.is_absolute() {
        return Err(PublisherError::UnsafePath);
    }
    let metadata = fs::symlink_metadata(path).map_err(PublisherError::Filesystem)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(PublisherError::UnsafePath);
    }
    fs::canonicalize(path).map_err(PublisherError::Filesystem)
}

fn canonical_executable(path: &Path) -> Result<PathBuf, PublisherError> {
    if !path.is_absolute() {
        return Err(PublisherError::UnsafePath);
    }
    let metadata = fs::symlink_metadata(path).map_err(PublisherError::Filesystem)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(PublisherError::UnsafePath);
    }
    fs::canonicalize(path).map_err(PublisherError::Filesystem)
}

fn trusted_directory(root: &Path, path: &Path) -> Result<PathBuf, PublisherError> {
    let path = trusted_path(root, path)?;
    fs::metadata(&path)
        .map_err(PublisherError::Filesystem)?
        .is_dir()
        .then_some(path)
        .ok_or(PublisherError::UnsafePath)
}

fn trusted_file(root: &Path, path: &Path) -> Result<PathBuf, PublisherError> {
    let path = trusted_path(root, path)?;
    fs::metadata(&path)
        .map_err(PublisherError::Filesystem)?
        .is_file()
        .then_some(path)
        .ok_or(PublisherError::UnsafePath)
}

fn trusted_path(root: &Path, path: &Path) -> Result<PathBuf, PublisherError> {
    if !path.is_absolute() || !path.starts_with(root) {
        return Err(PublisherError::UnsafePath);
    }
    let relative = path
        .strip_prefix(root)
        .map_err(|_| PublisherError::UnsafePath)?;
    if relative
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(PublisherError::UnsafePath);
    }
    let mut current = root.to_owned();
    for component in relative.components() {
        let Component::Normal(component) = component else {
            return Err(PublisherError::UnsafePath);
        };
        current.push(component);
        if fs::symlink_metadata(&current)
            .map_err(PublisherError::Filesystem)?
            .file_type()
            .is_symlink()
        {
            return Err(PublisherError::UnsafePath);
        }
    }
    let canonical = fs::canonicalize(path).map_err(PublisherError::Filesystem)?;
    canonical
        .starts_with(root)
        .then_some(canonical)
        .ok_or(PublisherError::UnsafePath)
}

fn safe_layout_file(layout: &Path, relative: &Path) -> Result<PathBuf, PublisherError> {
    trusted_file(layout, &layout.join(relative))
}

fn is_authentication_failure(stderr: &[u8]) -> bool {
    let lower = String::from_utf8_lossy(stderr).to_ascii_lowercase();
    ["unauthorized", "authentication", "token expired", "denied"]
        .iter()
        .any(|needle| lower.contains(needle))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LocalLayoutVersion {
    image_layout_version: String,
}

#[derive(Debug, Deserialize)]
struct LocalIndex {
    manifests: Vec<RemoteDescriptor>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RemoteDescriptor {
    media_type: String,
    digest: String,
    size: u64,
    #[serde(default)]
    artifact_type: Option<String>,
    #[serde(default)]
    platform: Option<RemotePlatform>,
    #[serde(default)]
    annotations: BTreeMap<String, String>,
}

impl RemoteDescriptor {
    fn to_domain(&self) -> Result<OciDescriptor, PublisherError> {
        OciDescriptor::new(
            Sha256Digest::parse(self.digest.clone()).map_err(PublisherError::Domain)?,
            self.size,
            OciMediaType::parse(self.media_type.clone()).map_err(PublisherError::Domain)?,
        )
        .map_err(PublisherError::Domain)
    }
}

type RemoteReferrerDescriptor = RemoteDescriptor;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RemotePlatform {
    #[serde(rename = "os")]
    operating_system: String,
    architecture: String,
    #[serde(default)]
    variant: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RemoteReferrers {
    manifests: Vec<RemoteReferrerDescriptor>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RemoteManifest {
    #[serde(default)]
    media_type: Option<String>,
    #[serde(default)]
    manifests: Vec<RemoteDescriptor>,
    #[serde(default)]
    subject: Option<RemoteSubject>,
    #[serde(default)]
    artifact_type: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RemoteSubject {
    digest: String,
}

/// Non-sensitive command launch failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CommandRunnerError {
    /// The configured executable could not be launched.
    #[error("OCI client executable could not be launched")]
    LaunchFailed,
}

/// Controlled publisher failure. No variant contains a bearer token or command output.
#[derive(Debug, thiserror::Error)]
pub enum PublisherError {
    /// The durable intent points at another registry authority.
    #[error("publication intent authority differs from publisher configuration")]
    AuthorityMismatch,
    /// The durable intent was already verified or approved.
    #[error("publication intent has already established immutable verification")]
    AlreadyVerified,
    /// The durable intent cannot be retried.
    #[error("publication intent is retired or missing")]
    NonRetryableIntent,
    /// A path was outside the configured root, relative, or contained a symlink.
    #[error("publisher input path is unsafe")]
    UnsafePath,
    /// A filesystem operation failed without exposing a sensitive path or payload.
    #[error("publisher filesystem operation failed")]
    Filesystem(#[source] std::io::Error),
    /// The local OCI layout root file was malformed.
    #[error("local OCI layout metadata is malformed")]
    MalformedLocalLayout,
    /// The local OCI index was malformed.
    #[error("local OCI index is malformed")]
    MalformedLocalIndex,
    /// The local index did not contain exactly the intended digest.
    #[error("local OCI layout does not contain the intended digest")]
    WrongLocalDigest,
    /// The local OCI layout holds more than one possible publication subject.
    #[error("local OCI layout has an ambiguous publication subject")]
    AmbiguousLocalLayout,
    /// The local OCI layout lacks the administrator-required immutable source tag.
    #[error("local OCI layout does not bind its source tag to the intent digest")]
    WrongLocalReferenceName,
    /// The local descriptor did not exactly match the durable publication intent.
    #[error("local OCI descriptor differs from publication intent")]
    WrongLocalDescriptor,
    /// A signature is required by policy but no trusted signature file was supplied.
    #[error("required signature evidence is absent")]
    MissingSignature,
    /// The OCI client could not be launched.
    #[error("OCI client command runner failed: {0}")]
    Runner(#[source] CommandRunnerError),
    /// Zot rejected the scoped bearer credential.
    #[error("registry authentication failed")]
    AuthenticationFailed,
    /// Direct bounded registry read-back failed without exposing response data.
    #[error("registry read-back failed")]
    RegistryRead(#[source] RegistryReadError),
    /// The configured registry read origin was not a credential-free HTTP(S) origin.
    #[error("registry read origin is unsafe")]
    UnsafeRegistryOrigin,
    /// A non-authentication OCI client command failed.
    #[error("OCI client command failed")]
    CommandFailed,
    /// Zot did not return a valid top-level descriptor.
    #[error("registry returned a malformed manifest descriptor")]
    MalformedRemoteDescriptor,
    /// Zot returned a descriptor different from the immutable intent.
    #[error("registry returned the wrong manifest descriptor")]
    WrongRemoteDescriptor,
    /// Zot returned a malformed manifest document.
    #[error("registry returned a malformed manifest")]
    MalformedRemoteManifest,
    /// Raw manifest bytes did not match the read-back descriptor.
    #[error("registry manifest bytes do not match their descriptor")]
    WrongRemoteManifestBytes,
    /// Zot returned a malformed or incomplete multi-platform index.
    #[error("registry returned a malformed multi-platform index")]
    MalformedRemoteIndex,
    /// Zot returned malformed referrer discovery data.
    #[error("registry returned malformed referrer discovery data")]
    MalformedReferrers,
    /// Required evidence was absent or duplicated in referrer discovery.
    #[error("registry referrer evidence is missing or duplicated: {0:?}")]
    MissingOrDuplicateReferrer(SupplyChainReferrerKind),
    /// A discovered evidence manifest did not link to the immutable subject.
    #[error("registry referrer points to another subject")]
    WrongReferrerSubject,
    /// A referrer descriptor or manifest differed from the discovered evidence.
    #[error("registry referrer descriptor or manifest is inconsistent")]
    WrongReferrerDescriptor,
    /// A domain invariant rejected local or remote data.
    #[error("registry domain verification failed: {0}")]
    Domain(#[source] RegistryValueError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use registry_domain::{
        NamespaceClaim, PlatformImageKey, PolicyVersion, PublicationIntentId, RegistryNamespace,
        RegistryOwner,
    };
    use registry_token::{
        AuthorizationDecision, IssuedToken, KeyId, RegistryService, RegistryTokenIssuer,
        RepositoryActions, RepositoryName, ScopeRequest, SigningKey, TokenIssuer, TokenLifetime,
        TokenSubject, UnixTimestamp,
    };
    use std::{
        collections::VecDeque,
        sync::{Arc, Mutex},
    };

    const B: char = 'b';
    const E: char = 'e';

    #[derive(Clone, Default)]
    struct ScriptedRunner {
        outputs: Arc<Mutex<VecDeque<CommandOutput>>>,
        seen: Arc<Mutex<Vec<CommandSpec>>>,
    }

    impl ScriptedRunner {
        fn with(outputs: Vec<CommandOutput>) -> Self {
            Self {
                outputs: Arc::new(Mutex::new(outputs.into())),
                seen: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn command_log(&self) -> Arc<Mutex<Vec<CommandSpec>>> {
            self.seen.clone()
        }
    }

    impl CommandRunner for ScriptedRunner {
        fn run(&self, command: &CommandSpec) -> Result<CommandOutput, CommandRunnerError> {
            self.seen.lock().expect("seen").push(command.clone());
            self.outputs
                .lock()
                .expect("outputs")
                .pop_front()
                .ok_or(CommandRunnerError::LaunchFailed)
        }
    }

    impl RegistryReadClient for ScriptedRunner {
        fn manifest_descriptor(
            &self,
            _namespace: &str,
            _digest: &Sha256Digest,
            _token: &BearerToken,
        ) -> Result<OciDescriptor, RegistryReadError> {
            let output = self.next_read_output()?;
            if !output.success {
                return Err(RegistryReadError::Failed);
            }
            let descriptor: RemoteDescriptor =
                serde_json::from_slice(&output.stdout).map_err(|_| RegistryReadError::Failed)?;
            descriptor
                .to_domain()
                .map_err(|_| RegistryReadError::Failed)
        }

        fn manifest_bytes(
            &self,
            _namespace: &str,
            _digest: &Sha256Digest,
            _token: &BearerToken,
        ) -> Result<Vec<u8>, RegistryReadError> {
            self.next_read_output().and_then(|output| {
                output
                    .success
                    .then_some(output.stdout)
                    .ok_or(RegistryReadError::Failed)
            })
        }

        fn referrers_bytes(
            &self,
            namespace: &str,
            subject: &Sha256Digest,
            token: &BearerToken,
        ) -> Result<Vec<u8>, RegistryReadError> {
            self.manifest_bytes(namespace, subject, token)
        }
    }

    impl ScriptedRunner {
        fn next_read_output(&self) -> Result<CommandOutput, RegistryReadError> {
            self.outputs
                .lock()
                .expect("outputs")
                .pop_front()
                .ok_or(RegistryReadError::Failed)
        }
    }

    fn scripted_publisher(
        configuration: PublisherConfiguration,
        runner: ScriptedRunner,
    ) -> ControlledOciPublisher<ScriptedRunner, ScriptedRunner> {
        ControlledOciPublisher::with_read_client(configuration, runner.clone(), runner)
    }

    fn digest(character: char) -> Sha256Digest {
        Sha256Digest::parse(format!("sha256:{}", character.to_string().repeat(64))).expect("digest")
    }

    fn descriptor(character: char, media_type: &str, size: u64) -> OciDescriptor {
        OciDescriptor::new(
            digest(character),
            size,
            OciMediaType::parse(media_type).expect("media"),
        )
        .expect("descriptor")
    }

    fn authority() -> RegistryAuthority {
        RegistryAuthority::parse("registry.example.test").expect("authority")
    }

    fn intent(expected: OciDescriptor) -> PublicationIntent {
        let owner = RegistryOwner::PlatformImage {
            image_key: PlatformImageKey::parse("rust-ubuntu").expect("key"),
        };
        let namespace = RegistryNamespace::for_owner(owner.clone());
        let reference =
            ImmutableManifestReference::new(authority(), namespace, expected.digest().clone());
        PublicationIntent::new(
            PublicationIntentId::new(),
            NamespaceClaim::new(owner),
            reference,
            expected,
            PolicyVersion::parse("test-v1").expect("policy"),
            SupplyChainPolicy::without_signature(),
        )
        .expect("intent")
    }

    fn token() -> IssuedToken {
        let service: RegistryService = authority().to_string().parse().expect("service");
        let mut grants = AuthorizationDecision::deny_all();
        grants.grant(
            "platform/images/rust-ubuntu"
                .parse::<RepositoryName>()
                .expect("repository"),
            RepositoryActions::pull_push(),
        );
        RegistryTokenIssuer::new(
            "https://forge.example.test/registry-token"
                .parse::<TokenIssuer>()
                .expect("issuer"),
            service.clone(),
            SigningKey::hs256("publisher-v1".parse::<KeyId>().expect("key id"), &[7; 32])
                .expect("signing key"),
            TokenLifetime::new(300).expect("lifetime"),
        )
        .issue(
            "publisher-worker".parse::<TokenSubject>().expect("subject"),
            &ScopeRequest::parse(
                service.as_str(),
                "repository:platform/images/rust-ubuntu:pull,push",
            )
            .expect("scope"),
            &grants,
            UnixTimestamp::new(1_700_000_000),
        )
        .expect("issued token")
    }

    fn setup() -> (
        tempfile::TempDir,
        PublisherConfiguration,
        PublicationMaterial,
        PublicationIntent,
    ) {
        let root = tempfile::tempdir().expect("temp root");
        let layouts = root.path().join("layouts");
        let credentials = root.path().join("credentials");
        fs::create_dir(&layouts).expect("layouts");
        fs::create_dir(&credentials).expect("credentials");
        let skopeo = executable(root.path(), "skopeo");
        let oras = executable(root.path(), "oras");
        let config =
            PublisherConfiguration::new(authority(), &layouts, &credentials, &skopeo, &oras)
                .expect("configuration");
        let image = layouts.join("image");
        let expected = create_layout(&image);
        let evidence = PublicationEvidenceFiles {
            sbom: write_file(&layouts, "sbom.json", b"sbom"),
            provenance: write_file(&layouts, "provenance.json", b"provenance"),
            scan: write_file(&layouts, "scan.json", b"scan"),
            signature: None,
        };
        (
            root,
            config,
            PublicationMaterial {
                layout: image,
                evidence,
            },
            intent(expected),
        )
    }

    fn executable(root: &Path, name: &str) -> PathBuf {
        let path = root.join(name);
        fs::write(&path, b"#!/bin/true\n").expect("executable");
        path
    }

    fn write_file(root: &Path, name: &str, contents: &[u8]) -> PathBuf {
        let path = root.join(name);
        fs::write(&path, contents).expect("file");
        path
    }

    fn create_layout(path: &Path) -> OciDescriptor {
        fs::create_dir(path).expect("layout");
        fs::create_dir_all(path.join("blobs/sha256")).expect("blobs");
        let platform = descriptor(B, OCI_MANIFEST_MEDIA_TYPE, 32);
        let bytes = format!(
            r#"{{"mediaType":"{OCI_INDEX_MEDIA_TYPE}","manifests":[{{"mediaType":"{}","digest":"{}","size":{},"platform":{{"os":"linux","architecture":"amd64"}}}}]}}"#,
            OCI_MANIFEST_MEDIA_TYPE,
            platform.digest(),
            platform.size()
        )
        .into_bytes();
        let actual = format!("sha256:{:x}", Sha256::digest(&bytes));
        let expected_size = u64::try_from(bytes.len()).expect("size");
        let reference_name =
            local_reference_tag(&Sha256Digest::parse(actual.clone()).expect("digest"));
        fs::write(path.join("oci-layout"), r#"{"imageLayoutVersion":"1.0.0"}"#)
            .expect("layout json");
        fs::write(
            path.join("index.json"),
            format!(r#"{{"manifests":[{{"mediaType":"{OCI_INDEX_MEDIA_TYPE}","digest":"{actual}","size":{expected_size},"annotations":{{"{OCI_REFERENCE_NAME_ANNOTATION}":"{reference_name}"}}}}]}}"#),
        )
        .expect("index");
        fs::write(
            path.join("blobs/sha256")
                .join(actual.trim_start_matches("sha256:")),
            bytes,
        )
        .expect("blob");
        OciDescriptor::new(
            Sha256Digest::parse(actual).expect("digest"),
            expected_size,
            OciMediaType::parse(OCI_INDEX_MEDIA_TYPE).expect("media type"),
        )
        .expect("descriptor")
    }

    fn successful_outputs(
        intent: &PublicationIntent,
        material: &PublicationMaterial,
    ) -> Vec<CommandOutput> {
        successful_outputs_for_subject(
            intent,
            material,
            intent.reference().digest(),
            material.evidence.signature.is_some(),
        )
    }

    fn successful_outputs_for_subject(
        intent: &PublicationIntent,
        material: &PublicationMaterial,
        referrer_subject: &Sha256Digest,
        include_signature: bool,
    ) -> Vec<CommandOutput> {
        let subject = intent.reference().digest();
        let descriptor = format!(
            r#"{{"mediaType":"{OCI_INDEX_MEDIA_TYPE}","digest":"{subject}","size":{}}}"#,
            intent.expected_manifest().size()
        );
        let index = fs::read(
            material
                .layout
                .join("blobs/sha256")
                .join(subject.as_str().trim_start_matches("sha256:")),
        )
        .expect("subject manifest");
        let mut artifact_types = vec![
            SBOM_ARTIFACT_TYPE,
            PROVENANCE_ARTIFACT_TYPE,
            SCAN_ARTIFACT_TYPE,
        ];
        if include_signature {
            artifact_types.push(SIGNATURE_ARTIFACT_TYPE);
        }
        let referrer_manifests = artifact_types.into_iter()
            .map(|artifact_type| {
                let bytes = format!(
                    r#"{{"mediaType":"{OCI_MANIFEST_MEDIA_TYPE}","artifactType":"{artifact_type}","subject":{{"digest":"{referrer_subject}"}}}}"#
                )
                .into_bytes();
                let descriptor = descriptor_for_bytes(&bytes, OCI_MANIFEST_MEDIA_TYPE);
                (artifact_type, descriptor, bytes)
            })
            .collect::<Vec<_>>();
        let referrers = referrer_manifests
            .iter()
            .map(|(artifact_type, descriptor, _)| {
                format!(
                    r#"{{"mediaType":"{}","digest":"{}","size":{},"artifactType":"{artifact_type}"}}"#,
                    descriptor.media_type(),
                    descriptor.digest(),
                    descriptor.size()
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        let mut outputs = vec![CommandOutput::success(Vec::new())];
        outputs.extend(
            std::iter::repeat_with(|| CommandOutput::success(Vec::new()))
                .take(referrer_manifests.len()),
        );
        outputs.push(CommandOutput::success(descriptor));
        outputs.push(CommandOutput::success(index));
        outputs.push(CommandOutput::success(format!(
            r#"{{"manifests":[{referrers}]}}"#
        )));
        for (_, descriptor, bytes) in referrer_manifests {
            outputs.push(CommandOutput::success(format!(
                r#"{{"mediaType":"{}","digest":"{}","size":{}}}"#,
                descriptor.media_type(),
                descriptor.digest(),
                descriptor.size()
            )));
            outputs.push(CommandOutput::success(bytes));
        }
        outputs
    }

    fn descriptor_for_bytes(bytes: &[u8], media_type: &str) -> OciDescriptor {
        OciDescriptor::new(
            Sha256Digest::parse(format!("sha256:{:x}", Sha256::digest(bytes))).expect("digest"),
            u64::try_from(bytes.len()).expect("size"),
            OciMediaType::parse(media_type).expect("media type"),
        )
        .expect("descriptor")
    }

    #[test]
    fn publishes_by_intent_digest_and_returns_verified_evidence() {
        let (_root, config, material, intent) = setup();
        let runner = ScriptedRunner::with(successful_outputs(&intent, &material));
        let publisher = scripted_publisher(config, runner);
        let token = token();
        let verified = publisher
            .publish(&intent, &material, token.token())
            .expect("verified");
        assert_eq!(verified.manifest(), intent.expected_manifest());
        assert_eq!(verified.platforms().len(), 1);
        assert_eq!(verified.evidence().referrers().len(), 3);
    }

    #[test]
    fn verifies_an_optional_signature_referrer_when_it_is_published() {
        let (_root, config, mut material, intent) = setup();
        material.evidence.signature = Some(write_file(
            material.layout.parent().expect("layout parent"),
            "signature.json",
            b"signature",
        ));
        let runner = ScriptedRunner::with(successful_outputs(&intent, &material));
        let publisher = scripted_publisher(config, runner);
        let issued = token();
        let verified = publisher
            .publish(&intent, &material, issued.token())
            .expect("verified");
        assert_eq!(verified.evidence().referrers().len(), 4);
    }

    #[test]
    fn interrupted_upload_is_retryable_without_local_mutation() {
        let (_root, config, material, intent) = setup();
        let publisher = scripted_publisher(
            config,
            ScriptedRunner::with(vec![CommandOutput::failure("connection reset")]),
        );
        let issued = token();
        assert!(matches!(
            publisher.publish(&intent, &material, issued.token()),
            Err(PublisherError::CommandFailed)
        ));
        assert_eq!(intent.state(), PublicationState::Pending);
    }

    #[test]
    fn rejects_malformed_layout_index_before_commands() {
        let (_root, config, material, intent) = setup();
        fs::write(material.layout.join("index.json"), b"not json").expect("rewrite");
        let publisher = scripted_publisher(config, ScriptedRunner::default());
        let issued = token();
        assert!(matches!(
            publisher.publish(&intent, &material, issued.token()),
            Err(PublisherError::MalformedLocalIndex)
        ));
    }

    #[test]
    fn rejects_malformed_subject_index_before_commands() {
        let (_root, config, material, _intent) = setup();
        let bytes = b"not an OCI index";
        let expected = descriptor_for_bytes(bytes, OCI_INDEX_MEDIA_TYPE);
        let tag = local_reference_tag(expected.digest());
        fs::write(
            material.layout.join("index.json"),
            format!(
                r#"{{"manifests":[{{"mediaType":"{OCI_INDEX_MEDIA_TYPE}","digest":"{}","size":{},"annotations":{{"{OCI_REFERENCE_NAME_ANNOTATION}":"{tag}"}}}}]}}"#,
                expected.digest(),
                expected.size()
            ),
        )
        .expect("index");
        fs::write(
            material
                .layout
                .join("blobs/sha256")
                .join(expected.digest().as_str().trim_start_matches("sha256:")),
            bytes,
        )
        .expect("blob");
        let publisher = scripted_publisher(config, ScriptedRunner::default());
        let issued = token();
        assert!(matches!(
            publisher.publish(&intent(expected), &material, issued.token()),
            Err(PublisherError::MalformedLocalIndex)
        ));
    }

    #[test]
    fn rejects_wrong_remote_digest() {
        let (_root, config, material, intent) = setup();
        let mut outputs = successful_outputs(&intent, &material);
        outputs[4] = CommandOutput::success(format!(
            r#"{{"mediaType":"{OCI_INDEX_MEDIA_TYPE}","digest":"{}","size":256}}"#,
            digest(E)
        ));
        let publisher = scripted_publisher(config, ScriptedRunner::with(outputs));
        let token = token();
        assert!(matches!(
            publisher.publish(&intent, &material, token.token()),
            Err(PublisherError::WrongRemoteDescriptor)
        ));
    }

    #[test]
    fn rejects_missing_or_wrong_subject_referrer() {
        let (_root, config, material, intent) = setup();
        let mut missing = successful_outputs(&intent, &material);
        missing[6] = CommandOutput::success(r#"{"manifests":[]}"#);
        let publisher = scripted_publisher(config.clone(), ScriptedRunner::with(missing));
        let issued = token();
        assert!(matches!(
            publisher.publish(&intent, &material, issued.token()),
            Err(PublisherError::MissingOrDuplicateReferrer(_))
        ));
        let wrong_subject = successful_outputs_for_subject(&intent, &material, &digest(E), false);
        let publisher = scripted_publisher(config, ScriptedRunner::with(wrong_subject));
        let issued = token();
        assert!(matches!(
            publisher.publish(&intent, &material, issued.token()),
            Err(PublisherError::WrongReferrerSubject)
        ));
    }

    #[test]
    fn classifies_expired_authentication_without_token_disclosure() {
        let (_root, config, material, intent) = setup();
        let runner =
            ScriptedRunner::with(vec![CommandOutput::failure("token expired: secret-token")]);
        let publisher = scripted_publisher(config, runner);
        let token = token();
        let error = publisher
            .publish(&intent, &material, token.token())
            .expect_err("auth error");
        assert!(matches!(error, PublisherError::AuthenticationFailed));
        assert!(!error.to_string().contains("secret-token"));
    }

    #[test]
    fn duplicate_retry_is_safe() {
        let (_root, config, material, intent) = setup();
        let mut outputs = successful_outputs(&intent, &material);
        outputs.extend(successful_outputs(&intent, &material));
        let publisher = scripted_publisher(config, ScriptedRunner::with(outputs));
        let issued = token();
        let first = publisher
            .publish(&intent, &material, issued.token())
            .expect("first verification");
        let second = publisher
            .publish(&intent, &material, issued.token())
            .expect("retry verification");
        assert_eq!(first, second);
    }

    #[test]
    fn rechecks_missing_content_without_republishing() {
        let (_root, config, material, intent) = setup();
        let outputs = successful_outputs(&intent, &material);
        let first = scripted_publisher(config.clone(), ScriptedRunner::with(outputs.clone()))
            .publish(&intent, &material, token().token())
            .expect("initial verification");
        let missing = intent
            .record_verified(first)
            .expect("verified")
            .approve()
            .expect("approved")
            .mark_missing()
            .expect("missing");
        let publisher = scripted_publisher(config, ScriptedRunner::with(outputs[4..].to_vec()));
        let verified = publisher
            .verify_existing(&missing, &material, token().token())
            .expect("reverified");
        assert_eq!(verified.manifest(), missing.expected_manifest());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_path_escape_and_symbolic_links() {
        use std::os::unix::fs::symlink;
        let (root, config, mut material, intent) = setup();
        material.layout = root.path().join("outside");
        fs::create_dir(&material.layout).expect("outside");
        let publisher = scripted_publisher(config.clone(), ScriptedRunner::default());
        let issued = token();
        assert!(matches!(
            publisher.publish(&intent, &material, issued.token()),
            Err(PublisherError::UnsafePath)
        ));
        let target = root.path().join("layouts/image");
        let link = root.path().join("layouts/link");
        symlink(target, &link).expect("link");
        material.layout = link;
        let publisher = scripted_publisher(config, ScriptedRunner::default());
        let issued = token();
        assert!(matches!(
            publisher.publish(&intent, &material, issued.token()),
            Err(PublisherError::UnsafePath)
        ));
    }

    #[test]
    fn commands_and_debug_redact_bearer_token() {
        let (_root, config, material, intent) = setup();
        let runner = ScriptedRunner::with(successful_outputs(&intent, &material));
        let commands = runner.command_log();
        let publisher = scripted_publisher(config, runner);
        let issued = token();
        let secret = issued.token().as_str().to_owned();
        publisher
            .publish(&intent, &material, issued.token())
            .expect("verified");
        let debug = format!("{:?}", issued.token());
        assert!(!debug.contains(&secret));
        for command in commands.lock().expect("commands").iter() {
            assert!(!format!("{command:?}").contains(&secret));
            if command
                .arguments()
                .iter()
                .any(|argument| argument.to_string_lossy().contains(&secret))
            {
                let token_position = command
                    .arguments()
                    .iter()
                    .position(|argument| argument.to_string_lossy().contains(&secret))
                    .expect("bearer token argument");
                assert_eq!(command.sensitive_argument_positions, vec![token_position]);
            }
            assert!(
                command
                    .environment_entries()
                    .iter()
                    .all(|(_, value)| !value.to_string_lossy().contains(&secret))
            );
        }
    }
}
