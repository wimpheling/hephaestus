use builder_catalog_domain::OciImageId;
use registry_domain::{
    ImmutableManifestReference, NamespaceClaim, OciDescriptor, OciMediaType, PlatformDescriptor,
    PlatformImageKey, PolicyVersion, PublicationIntent, PublicationIntentId, RegistryAuthority,
    RegistryOwner, Sha256Digest, SupplyChainEvidence, SupplyChainPolicy, SupplyChainReferrer,
    SupplyChainReferrerKind, VerifiedPublication,
};

pub fn intent() -> PublicationIntent {
    let owner = RegistryOwner::PlatformImage {
        image_key: PlatformImageKey::parse(format!("test-{}", uuid::Uuid::new_v4().simple()))
            .expect("platform key"),
    };
    let claim = NamespaceClaim::new(owner);
    let digest = digest('a');
    let reference = ImmutableManifestReference::new(
        RegistryAuthority::parse("registry.example").expect("authority"),
        claim.namespace().clone(),
        digest.clone(),
    );
    PublicationIntent::new(
        PublicationIntentId::new(),
        claim,
        reference,
        descriptor(digest, 100, OciMediaType::IMAGE_INDEX),
        PolicyVersion::parse("v1").expect("policy"),
        SupplyChainPolicy::without_signature(),
    )
    .expect("intent")
}

pub fn project_intent(project_id: uuid::Uuid, image_id: OciImageId) -> PublicationIntent {
    let owner = RegistryOwner::RepositoryOciImage {
        project_id: forge_domain::ProjectId::from_uuid(project_id),
        image_id,
    };
    let claim = NamespaceClaim::new(owner);
    let digest = digest('f');
    let reference = ImmutableManifestReference::new(
        RegistryAuthority::parse("registry.example").expect("authority"),
        claim.namespace().clone(),
        digest.clone(),
    );
    PublicationIntent::new(
        PublicationIntentId::new(),
        claim,
        reference,
        descriptor(digest, 100, OciMediaType::IMAGE_INDEX),
        PolicyVersion::parse("v1").expect("policy"),
        SupplyChainPolicy::without_signature(),
    )
    .expect("project intent")
}

pub fn verification(intent: &PublicationIntent) -> VerifiedPublication {
    let subject = intent.reference().digest().clone();
    let platform = PlatformDescriptor::new(
        descriptor(digest('b'), 99, OciMediaType::IMAGE_MANIFEST),
        "linux",
        "amd64",
        None,
    )
    .expect("platform");
    let evidence = SupplyChainEvidence::new(
        subject.clone(),
        vec![
            referrer(SupplyChainReferrerKind::Sbom, subject.clone(), 'c'),
            referrer(SupplyChainReferrerKind::Provenance, subject.clone(), 'd'),
            referrer(SupplyChainReferrerKind::Scan, subject, 'e'),
        ],
    )
    .expect("evidence");
    VerifiedPublication::new(
        intent.reference(),
        intent.expected_manifest().clone(),
        vec![platform],
        evidence,
    )
    .expect("verification")
}

fn referrer(
    kind: SupplyChainReferrerKind,
    subject: Sha256Digest,
    byte: char,
) -> SupplyChainReferrer {
    SupplyChainReferrer::new(
        kind,
        subject,
        descriptor(digest(byte), 42, "application/vnd.in-toto+json"),
        OciMediaType::parse("application/vnd.in-toto+json").expect("artifact type"),
    )
}

fn descriptor(digest: Sha256Digest, size: u64, media_type: &str) -> OciDescriptor {
    OciDescriptor::new(
        digest,
        size,
        OciMediaType::parse(media_type).expect("media type"),
    )
    .expect("descriptor")
}

fn digest(byte: char) -> Sha256Digest {
    Sha256Digest::parse(format!("sha256:{}", byte.to_string().repeat(64))).expect("digest")
}
