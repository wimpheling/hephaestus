use crate::remote::RemoteDescriptor;
pub(super) use crate::{
    CommandOutput, CommandRunner, CommandRunnerError, CommandSpec, ControlledOciPublisher,
    PublicationEvidenceFiles, PublicationMaterial, PublisherConfiguration, PublisherError,
    RegistryReadClient, RegistryReadError, constants::*, paths::local_reference_tag,
};
pub(super) use registry_domain::{
    ImmutableManifestReference, NamespaceClaim, OciDescriptor, OciMediaType, PlatformImageKey,
    PolicyVersion, PublicationIntent, PublicationIntentId, PublicationState, RegistryAuthority,
    RegistryNamespace, RegistryOwner, Sha256Digest, SupplyChainPolicy,
};
pub(super) use registry_token::{
    AuthorizationDecision, BearerToken, IssuedToken, KeyId, RegistryService, RegistryTokenIssuer,
    RepositoryActions, RepositoryName, ScopeRequest, SigningKey, TokenIssuer, TokenLifetime,
    TokenSubject, UnixTimestamp,
};
pub(super) use sha2::{Digest, Sha256};
pub(super) use std::{
    collections::VecDeque,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

pub(super) const B: char = 'b';
pub(super) const E: char = 'e';

#[derive(Clone, Default)]
pub(super) struct ScriptedRunner {
    outputs: Arc<Mutex<VecDeque<CommandOutput>>>,
    seen: Arc<Mutex<Vec<CommandSpec>>>,
}

impl ScriptedRunner {
    pub(super) fn with(outputs: Vec<CommandOutput>) -> Self {
        Self {
            outputs: Arc::new(Mutex::new(outputs.into())),
            seen: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub(super) fn command_log(&self) -> Arc<Mutex<Vec<CommandSpec>>> {
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
    pub(super) fn next_read_output(&self) -> Result<CommandOutput, RegistryReadError> {
        self.outputs
            .lock()
            .expect("outputs")
            .pop_front()
            .ok_or(RegistryReadError::Failed)
    }
}

pub(super) fn scripted_publisher(
    configuration: PublisherConfiguration,
    runner: ScriptedRunner,
) -> ControlledOciPublisher<ScriptedRunner, ScriptedRunner> {
    ControlledOciPublisher::with_read_client(configuration, runner.clone(), runner)
}

pub(super) fn digest(character: char) -> Sha256Digest {
    Sha256Digest::parse(format!("sha256:{}", character.to_string().repeat(64))).expect("digest")
}

pub(super) fn descriptor(character: char, media_type: &str, size: u64) -> OciDescriptor {
    OciDescriptor::new(
        digest(character),
        size,
        OciMediaType::parse(media_type).expect("media"),
    )
    .expect("descriptor")
}

pub(super) fn authority() -> RegistryAuthority {
    RegistryAuthority::parse("registry.example.test").expect("authority")
}

pub(super) fn intent(expected: OciDescriptor) -> PublicationIntent {
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

pub(super) fn token() -> IssuedToken {
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

pub(super) fn setup() -> (
    tempfile::TempDir,
    PublisherConfiguration,
    PublicationMaterial,
    PublicationIntent,
) {
    let root = tempfile::tempdir().expect("temp root");
    let layouts = root.path().join("layouts");
    let evidence_root = root.path().join("evidence");
    let credentials = root.path().join("credentials");
    fs::create_dir(&layouts).expect("layouts");
    fs::create_dir(&evidence_root).expect("evidence root");
    fs::create_dir(&credentials).expect("credentials");
    let skopeo = executable(root.path(), "skopeo");
    let oras = executable(root.path(), "oras");
    let config = PublisherConfiguration::new(
        authority(),
        &layouts,
        &evidence_root,
        &credentials,
        &skopeo,
        &oras,
    )
    .expect("configuration");
    let image = layouts.join("image");
    let expected = create_layout(&image);
    let evidence = PublicationEvidenceFiles {
        sbom: write_file(&evidence_root, "sbom.json", b"sbom"),
        provenance: write_file(&evidence_root, "provenance.json", b"provenance"),
        scan: write_file(&evidence_root, "scan.json", b"scan"),
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

pub(super) fn executable(root: &Path, name: &str) -> PathBuf {
    let path = root.join(name);
    fs::write(&path, b"#!/bin/true\n").expect("executable");
    path
}

pub(super) fn write_file(root: &Path, name: &str, contents: &[u8]) -> PathBuf {
    let path = root.join(name);
    fs::write(&path, contents).expect("file");
    path
}

pub(super) fn create_layout(path: &Path) -> OciDescriptor {
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
    let reference_name = local_reference_tag(&Sha256Digest::parse(actual.clone()).expect("digest"));
    fs::write(path.join("oci-layout"), r#"{"imageLayoutVersion":"1.0.0"}"#).expect("layout json");
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
