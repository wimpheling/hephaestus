use super::{catalog_validation::parse_image_catalog_manifest, inspection::InspectionTarget};
use authz_domain::{ObjectType, Permission};
use std::path::Path;

const TEST_DIGEST: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

#[test]
fn inspection_targets_have_closed_authorization_policies() {
    assert_eq!(InspectionTarget::Release.object_type(), ObjectType::Release);
    assert_eq!(InspectionTarget::Release.permission(), Permission::CanRead);
    assert_eq!(
        InspectionTarget::AgentInstance.object_type(),
        ObjectType::AgentInstance
    );
    assert_eq!(
        InspectionTarget::AgentInstance.permission(),
        Permission::CanRead
    );
    assert_eq!(InspectionTarget::Secret.object_type(), ObjectType::Secret);
    assert_eq!(
        InspectionTarget::Secret.permission(),
        Permission::InspectMetadata
    );
}

fn manifest_json(reference: &str) -> String {
    r#"{
                "schema_version": 1,
                "images": [{
                    "id": "20000000-0000-4000-8000-000000000010",
                    "key": "ubuntu-native",
                    "display_name": "Ubuntu native image",
                    "image_reference": "__REFERENCE__",
                    "toolchains": [{"name":"shell","version":"ubuntu-24.04"}],
                    "architectures": ["x86_64"],
                    "availability_state": "available",
                    "provenance": {"source":"attestation://test/ubuntu-native"},
                    "platform_policy_version": "image/v1"
                }]
            }"#
    .replace("__REFERENCE__", reference)
}

#[test]
fn catalog_manifest_requires_explicit_digest_pinned_records() {
    let reference = format!("registry.example/ubuntu@sha256:{TEST_DIGEST}");
    let manifest = manifest_json(&reference);
    let parsed = parse_image_catalog_manifest(&manifest, Path::new("test.json"))
        .expect("manifest should be valid");
    assert_eq!(parsed.schema_version, 1);
    assert_eq!(parsed.images[0].key, "ubuntu-native");
    assert_eq!(parsed.images[0].image_reference, reference);
    assert_eq!(parsed.images[0].role, "execution");
}

#[test]
fn catalog_manifest_accepts_platform_operation_images() {
    let reference = format!("registry.example/oci-builder@sha256:{TEST_DIGEST}");
    let manifest = manifest_json(&reference).replace(
            "\"availability_state\": \"available\",",
            "\"availability_state\": \"available\",\n                    \"role\": \"platform_operation\",",
        );
    let parsed = parse_image_catalog_manifest(&manifest, Path::new("test.json"))
        .expect("platform operation image should be valid");
    assert_eq!(parsed.images[0].role, "platform_operation");
}

#[test]
fn catalog_manifest_rejects_unpinned_references() {
    let tagged = manifest_json("registry.example/ubuntu:24.04");
    assert!(parse_image_catalog_manifest(&tagged, Path::new("test.json")).is_err());
}
