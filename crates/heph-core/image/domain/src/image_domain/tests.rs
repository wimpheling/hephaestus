use super::{
    AvailabilityState, ImageKey, ImageProvenance, ImageRole, ImageSelectionError, OciImage,
    OciImageId, OciImageReference, Toolchain,
};

fn image(role: ImageRole) -> OciImage {
    OciImage {
        id: OciImageId::new(),
        key: ImageKey::parse("oci-builder-ubuntu").expect("valid key"),
        display_name: String::from("OCI builder Ubuntu"),
        image_reference: OciImageReference::parse(format!(
            "registry.example/platform/oci-builder@sha256:{}",
            "a".repeat(64)
        ))
        .expect("valid immutable reference"),
        toolchains: vec![Toolchain {
            name: String::from("buildah"),
            version: String::from("1.43.2"),
        }],
        architectures: vec![String::from("x86_64")],
        availability: AvailabilityState::Available,
        role,
        provenance: ImageProvenance {
            source: String::from("attestation://platform/oci-builder"),
            signature: None,
            sbom: None,
        },
        platform_policy_version: String::from("image/v1"),
    }
}

#[test]
fn platform_operation_image_is_not_tenant_selectable() {
    assert_eq!(
        image(ImageRole::PlatformOperation).resolve(),
        Err(ImageSelectionError::PlatformOperationOnly)
    );
}

#[test]
fn execution_image_remains_selectable() {
    assert!(image(ImageRole::Execution).resolve().is_ok());
}
