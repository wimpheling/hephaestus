use crate::publication::repository_image_claim;
use builder_catalog_domain::OciImageId;
use registry_domain::{
    ImmutableManifestReference, OciDescriptor, OciMediaType, PolicyVersion, PublicationIntent,
    PublicationIntentId, RegistryAuthority, Sha256Digest, SupplyChainPolicy,
};
use uuid::Uuid;

#[test]
fn namespace_uses_only_opaque_project_and_image_ids() {
    let project_id = Uuid::from_u128(1);
    let image_id = OciImageId::from_uuid(Uuid::from_u128(2));
    let claim = repository_image_claim(project_id, image_id).expect("namespace claim");
    assert_eq!(
        claim.namespace().as_str(),
        "projects/00000000-0000-0000-0000-000000000001/repository-images/00000000-0000-0000-0000-000000000002"
    );
}

#[test]
fn project_changes_produce_a_different_namespace_for_the_same_image() {
    let image_id = OciImageId::from_uuid(Uuid::from_u128(2));
    let first =
        repository_image_claim(Uuid::from_u128(1), image_id).expect("first namespace claim");
    let second =
        repository_image_claim(Uuid::from_u128(3), image_id).expect("second namespace claim");
    assert_ne!(first.namespace(), second.namespace());
}

#[test]
fn cross_project_intent_mismatch_is_rejected_before_it_reaches_zot() {
    let image_id = OciImageId::from_uuid(Uuid::from_u128(2));
    let first =
        repository_image_claim(Uuid::from_u128(1), image_id).expect("first namespace claim");
    let second =
        repository_image_claim(Uuid::from_u128(3), image_id).expect("second namespace claim");
    let descriptor = OciDescriptor::new(
        Sha256Digest::parse(format!("sha256:{}", "a".repeat(64))).expect("digest"),
        42,
        OciMediaType::parse(OciMediaType::IMAGE_INDEX).expect("media type"),
    )
    .expect("descriptor");
    let reference = ImmutableManifestReference::new(
        RegistryAuthority::parse("registry.example").expect("authority"),
        second.namespace().clone(),
        descriptor.digest().clone(),
    );
    assert!(
        PublicationIntent::new(
            PublicationIntentId::from_uuid(Uuid::from_u128(4)),
            first,
            reference,
            descriptor,
            PolicyVersion::parse("image-v1").expect("policy version"),
            SupplyChainPolicy::without_signature(),
        )
        .is_err()
    );
}

#[test]
fn interrupted_publication_intents_return_to_the_retryable_pending_state() {
    let claim = repository_image_claim(
        Uuid::from_u128(1),
        OciImageId::from_uuid(Uuid::from_u128(2)),
    )
    .expect("namespace claim");
    let descriptor = OciDescriptor::new(
        Sha256Digest::parse(format!("sha256:{}", "a".repeat(64))).expect("digest"),
        42,
        OciMediaType::parse(OciMediaType::IMAGE_INDEX).expect("media type"),
    )
    .expect("descriptor");
    let reference = ImmutableManifestReference::new(
        RegistryAuthority::parse("registry.example").expect("authority"),
        claim.namespace().clone(),
        descriptor.digest().clone(),
    );
    let intent = PublicationIntent::new(
        PublicationIntentId::from_uuid(Uuid::from_u128(4)),
        claim,
        reference,
        descriptor,
        PolicyVersion::parse("image-v1").expect("policy version"),
        SupplyChainPolicy::without_signature(),
    )
    .expect("intent")
    .begin_publishing()
    .expect("publishing")
    .retry()
    .expect("retry");
    assert!(matches!(
        intent.state(),
        registry_domain::PublicationState::Pending
    ));
}
