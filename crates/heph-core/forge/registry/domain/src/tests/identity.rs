use super::support::*;
use crate::{
    ImmutableManifestReference, NamespaceClaim, OciMediaType, PlatformImageKey, PolicyVersion,
    PublicationIntent, PublicationIntentId, RegistryAuthority, RegistryNamespace, RegistryOwner,
    RegistryOwnershipError, Sha256Digest, SupplyChainPolicy,
};
use builder_catalog_domain::OciImageId;
use forge_domain::ProjectId;
use runtime_types::ReleaseAgentId;
use uuid::Uuid;

#[test]
fn canonicalizes_all_supported_namespace_shapes() {
    let platform = RegistryNamespace::for_owner(RegistryOwner::PlatformImage {
        image_key: PlatformImageKey::parse("rust-ubuntu").expect("key"),
    });
    assert_eq!(platform.as_str(), "platform/images/rust-ubuntu");
    assert_eq!(RegistryNamespace::parse(platform.to_string()), Ok(platform));

    let repository = RegistryNamespace::for_owner(owner());
    assert_eq!(
        repository.as_str(),
        "projects/00000000-0000-0000-0000-000000000001/repository-images/00000000-0000-0000-0000-000000000002"
    );
    assert_eq!(
        RegistryNamespace::parse(repository.to_string()),
        Ok(repository)
    );

    let release = RegistryNamespace::for_owner(RegistryOwner::ReleaseAgent {
        project_id: ProjectId::from_uuid(Uuid::from_u128(1)),
        release_agent_id: ReleaseAgentId::from_uuid(Uuid::from_u128(4)),
    });
    assert_eq!(
        release.as_str(),
        "projects/00000000-0000-0000-0000-000000000001/release-agents/00000000-0000-0000-0000-000000000004"
    );
    assert_eq!(RegistryNamespace::parse(release.to_string()), Ok(release));
}

#[test]
fn rejects_noncanonical_paths_and_immutable_references() {
    assert!(
        RegistryNamespace::parse("projects/00000000-0000-0000-0000-000000000001/images/x").is_err()
    );
    assert!(RegistryNamespace::parse("platform/images/Rust").is_err());
    assert!(RegistryNamespace::parse("projects/{00000000-0000-0000-0000-000000000001}/repository-images/00000000-0000-0000-0000-000000000002").is_err());
    assert!(RegistryAuthority::parse("https://registry.example.test").is_err());
    assert!(RegistryAuthority::parse("Registry.example.test").is_err());
    assert!(
        ImmutableManifestReference::parse("registry.example.test/platform/images/rust:latest")
            .is_err()
    );
    assert!(
        ImmutableManifestReference::parse("registry.example.test/platform/images/rust@sha512:abc")
            .is_err()
    );
}

#[test]
fn accepts_only_lowercase_sha256_digests() {
    assert!(Sha256Digest::parse(format!("sha256:{}", "a".repeat(64))).is_ok());
    assert!(Sha256Digest::parse(format!("sha256:{}", "A".repeat(64))).is_err());
    assert!(Sha256Digest::parse(format!("sha256:{}", "g".repeat(64))).is_err());
    assert!(Sha256Digest::parse(format!("sha256:{}", "a".repeat(63))).is_err());
    assert!(Sha256Digest::parse(format!("sha512:{}", "a".repeat(64))).is_err());
}

#[test]
fn ownership_is_exact_and_cross_project_is_denied() {
    let claim = NamespaceClaim::new(owner());
    let same_namespace = RegistryNamespace::for_owner(owner());
    assert!(claim.assert_owns(&same_namespace).is_ok());

    let other_namespace = RegistryNamespace::for_owner(RegistryOwner::RepositoryOciImage {
        project_id: ProjectId::from_uuid(Uuid::from_u128(9)),
        image_id: OciImageId::from_uuid(Uuid::from_u128(2)),
    });
    assert_eq!(
        claim.assert_owns(&other_namespace),
        Err(RegistryOwnershipError::NamespaceMismatch)
    );
    assert!(
        PublicationIntent::new(
            PublicationIntentId::new(),
            claim,
            ImmutableManifestReference::new(authority(), other_namespace, digest(A)),
            descriptor(A, OciMediaType::IMAGE_INDEX),
            PolicyVersion::parse("registry/v1").expect("policy"),
            SupplyChainPolicy::default(),
        )
        .is_err()
    );
}
