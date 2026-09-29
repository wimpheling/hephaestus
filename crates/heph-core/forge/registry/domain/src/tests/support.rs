use crate::{
    ImmutableManifestReference, NamespaceClaim, OciDescriptor, OciMediaType, PlatformDescriptor,
    PolicyVersion, PublicationIntent, PublicationIntentId, RegistryAuthority, RegistryNamespace,
    RegistryOwner, Sha256Digest, SupplyChainEvidence, SupplyChainPolicy, SupplyChainReferrer,
    SupplyChainReferrerKind, VerifiedPublication,
};
use builder_catalog_domain::OciImageId;
use forge_domain::ProjectId;
use uuid::Uuid;

pub const A: char = 'a';
pub const B: char = 'b';
pub const C: char = 'c';
pub const D: char = 'd';
pub const E: char = 'e';

pub fn digest(character: char) -> Sha256Digest {
    Sha256Digest::parse(format!("sha256:{}", character.to_string().repeat(64)))
        .expect("test digest")
}

pub fn authority() -> RegistryAuthority {
    RegistryAuthority::parse("registry.example.test:5443").expect("authority")
}

pub fn owner() -> RegistryOwner {
    RegistryOwner::RepositoryOciImage {
        project_id: ProjectId::from_uuid(Uuid::from_u128(1)),
        image_id: OciImageId::from_uuid(Uuid::from_u128(2)),
    }
}

pub fn reference() -> ImmutableManifestReference {
    ImmutableManifestReference::new(
        authority(),
        RegistryNamespace::for_owner(owner()),
        digest(A),
    )
}

pub fn media_type(value: &str) -> OciMediaType {
    OciMediaType::parse(value).expect("media type")
}

pub fn descriptor(character: char, media_type_text: &str) -> OciDescriptor {
    OciDescriptor::new(digest(character), 42, media_type(media_type_text)).expect("descriptor")
}

pub fn verification(reference: &ImmutableManifestReference) -> VerifiedPublication {
    let subject = reference.digest().clone();
    let referrers = vec![
        SupplyChainReferrer::new(
            SupplyChainReferrerKind::Sbom,
            subject.clone(),
            descriptor(B, "application/spdx+json"),
            media_type("application/spdx+json"),
        ),
        SupplyChainReferrer::new(
            SupplyChainReferrerKind::Provenance,
            subject.clone(),
            descriptor(C, "application/vnd.in-toto+json"),
            media_type("application/vnd.in-toto+json"),
        ),
        SupplyChainReferrer::new(
            SupplyChainReferrerKind::Scan,
            subject.clone(),
            descriptor(D, "application/vnd.hephaestus.scan.v1+json"),
            media_type("application/vnd.hephaestus.scan.v1+json"),
        ),
    ];
    VerifiedPublication::new(
        reference,
        descriptor(A, OciMediaType::IMAGE_INDEX),
        vec![
            PlatformDescriptor::new(
                descriptor(E, OciMediaType::IMAGE_MANIFEST),
                "linux",
                "amd64",
                None,
            )
            .expect("platform"),
        ],
        SupplyChainEvidence::new(subject, referrers).expect("evidence"),
    )
    .expect("verification")
}

pub fn intent() -> PublicationIntent {
    let reference = reference();
    PublicationIntent::new(
        PublicationIntentId::from_uuid(Uuid::from_u128(3)),
        NamespaceClaim::new(owner()),
        reference,
        descriptor(A, OciMediaType::IMAGE_INDEX),
        PolicyVersion::parse("registry/v1").expect("policy version"),
        SupplyChainPolicy::default(),
    )
    .expect("intent")
}
