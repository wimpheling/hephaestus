use crate::{
    constants::{
        BUILDER_SCRATCH_DISK_ID, BUILDER_SCRATCH_GUEST_PATH, PLATFORM_OCI_BUILDER_ENV,
        PLATFORM_OCI_VERIFIER_ENV, ScratchDisk, VERIFIER_SYFT_CACHE_ENV, VERIFIER_SYFT_CACHE_PATH,
        VERIFIER_SYFT_UPDATE_ENV, VERIFIER_TRIVY_CACHE_ENV, VERIFIER_TRIVY_CACHE_PATH,
    },
    filesystem::prepare_scratch_disk,
    output::{layout_index_descriptor, normalize_layout_to_single_index},
    specs::{builder_vm_spec, verifier_vm_spec},
};
use builder_catalog_domain::{OciImageId, OciImageReference};
use registry_domain::OciMediaType;
use sha2::{Digest, Sha256};
use std::{fs, os::unix::fs::symlink, path::PathBuf, process::Command};
use uuid::Uuid;
use vm_trait::{NetworkMode, RootFilesystem, VmResources};

#[test]
fn builder_and_verifier_specs_preserve_the_vm_security_boundary() {
    let temporary = tempfile::tempdir().expect("temporary root");
    let checkout = temporary.path().join("checkout");
    let base = temporary.path().join("base");
    let candidate = temporary.path().join("candidate");
    let verification = temporary.path().join("verification");
    fs::create_dir_all(&checkout).expect("checkout");
    fs::create_dir(&base).expect("base");
    let request = oci_builder_worker::IsolatedOciBuild {
        job_id: Uuid::from_u128(1),
        image_id: OciImageId::from_uuid(Uuid::from_u128(2)),
        project_id: Uuid::from_u128(3),
        dockerfile: checkout.join("Dockerfile"),
        checkout_root: checkout.clone(),
        context: checkout.clone(),
        base_oci_layout: base.clone(),
        base_reference: OciImageReference::parse(
            "registry.example/platform/images/ubuntu-native@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_owned(),
        )
        .expect("base reference"),
        network_disabled: true,
        ambient_credentials_disabled: true,
    };
    let resources = VmResources {
        vcpus: 1,
        memory_mib: 256,
    };
    let scratch = ScratchDisk {
        path: temporary.path().join("scratch.raw"),
        filesystem_uuid: Uuid::from_u128(4),
    };
    let builder = builder_vm_spec(
        &request,
        &candidate,
        &scratch,
        &RootFilesystem::Directory {
            host_path: PathBuf::from("/platform/oci-builder"),
        },
        &resources,
    )
    .expect("builder spec");
    let verifier = verifier_vm_spec(
        &request,
        &candidate,
        &verification,
        &RootFilesystem::Directory {
            host_path: PathBuf::from("/platform/oci-verifier"),
        },
        &resources,
    );

    assert_builder_boundary(&builder, &request, &checkout, &base, &candidate, &scratch);
    assert_verifier_boundary(&verifier, &request, &candidate, &verification);
}

#[test]
fn canonicalized_formatter_creates_an_ext4_scratch_disk() {
    let temporary = tempfile::tempdir().expect("temporary scratch root");
    let target = ["/usr/sbin/mke2fs", "/usr/bin/mke2fs"]
        .into_iter()
        .map(PathBuf::from)
        .find(|path| path.is_file())
        .expect("mke2fs must be installed");
    let link = temporary.path().join("mkfs.ext4");
    symlink(&target, &link).expect("temporary formatter symlink");
    let canonical = fs::canonicalize(&link).expect("canonical temporary formatter");
    assert!(
        !fs::symlink_metadata(&canonical)
            .expect("canonical formatter metadata")
            .file_type()
            .is_symlink()
    );

    let scratch = prepare_scratch_disk(temporary.path(), &canonical, Uuid::from_u128(5))
        .expect("format canonicalized scratch disk");
    let output = Command::new("blkid")
        .args(["-s", "TYPE", "-o", "value"])
        .arg(&scratch.path)
        .output()
        .expect("inspect scratch filesystem type");
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "ext4");
}

#[test]
fn normalizes_the_builder_manifest_into_a_single_platform_index() {
    let temporary = tempfile::tempdir().expect("temporary root");
    let layout = temporary.path().join("layout");
    let blob_root = layout.join("blobs/sha256");
    fs::create_dir_all(&blob_root).expect("blob root");
    let manifest = br#"{"schemaVersion":2}"#;
    let manifest_digest = format!("sha256:{:x}", Sha256::digest(manifest));
    fs::write(
        blob_root.join(manifest_digest.trim_start_matches("sha256:")),
        manifest,
    )
    .expect("manifest blob");
    fs::write(
        layout.join("index.json"),
        serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 2,
            "manifests": [{
                "mediaType": OciMediaType::IMAGE_MANIFEST,
                "digest": manifest_digest,
                "size": manifest.len(),
                "annotations": { "org.opencontainers.image.ref.name": "latest" },
            }],
        }))
        .expect("index bytes"),
    )
    .expect("index");

    normalize_layout_to_single_index(&layout).expect("normalize layout");
    let descriptor = layout_index_descriptor(&layout).expect("platform index descriptor");

    assert!(descriptor.media_type().is_image_index());
    let index: serde_json::Value =
        serde_json::from_slice(&fs::read(layout.join("index.json")).expect("normalized index"))
            .expect("index JSON");
    assert!(
        index["manifests"][0]["annotations"]["org.opencontainers.image.ref.name"]
            .as_str()
            .is_some_and(|tag| tag.starts_with("heph-sha256-"))
    );
}

fn assert_builder_boundary(
    builder: &vm_trait::VmSpec,
    request: &oci_builder_worker::IsolatedOciBuild,
    checkout: &std::path::Path,
    base: &std::path::Path,
    candidate: &std::path::Path,
    scratch: &ScratchDisk,
) {
    assert!(matches!(builder.network, NetworkMode::Disabled));
    assert!(builder.runtime_authority.is_none());
    assert_eq!(builder.command.program, "/usr/libexec/hephaestus/oci-build");
    assert_eq!(builder.command.env[PLATFORM_OCI_BUILDER_ENV], "1");
    assert_eq!(
        builder.command.args,
        ["/workspace/source/Dockerfile", "/workspace/source"]
    );
    assert_eq!(builder.labels["hephaestus.kind"], "repository_oci_builder");
    assert_eq!(
        builder.labels["hephaestus.oci-job-id"],
        request.job_id.to_string()
    );
    assert_eq!(builder.mounts.len(), 3);
    assert!(builder.mounts[0].read_only);
    assert!(builder.mounts[1].read_only);
    assert!(!builder.mounts[2].read_only);
    assert_eq!(builder.mounts[0].host_path, checkout);
    assert_eq!(builder.mounts[1].host_path, base);
    assert_eq!(builder.mounts[2].host_path, candidate);
    assert_eq!(builder.disks.len(), 1);
    assert_eq!(builder.disks[0].id, BUILDER_SCRATCH_DISK_ID);
    assert_eq!(builder.disks[0].host_path, scratch.path);
    assert!(!builder.disks[0].read_only);
    assert_eq!(
        builder.labels["hephaestus.oci-scratch.filesystem-uuid"],
        scratch.filesystem_uuid.to_string()
    );
    assert_eq!(
        builder.labels["hephaestus.oci-scratch.mount-path"],
        BUILDER_SCRATCH_GUEST_PATH
    );
}

fn assert_verifier_boundary(
    verifier: &vm_trait::VmSpec,
    request: &oci_builder_worker::IsolatedOciBuild,
    candidate: &std::path::Path,
    verification: &std::path::Path,
) {
    assert!(matches!(verifier.network, NetworkMode::Disabled));
    assert!(verifier.runtime_authority.is_none());
    assert_eq!(
        verifier.command.env[VERIFIER_TRIVY_CACHE_ENV],
        VERIFIER_TRIVY_CACHE_PATH
    );
    assert_eq!(verifier.command.env[VERIFIER_SYFT_UPDATE_ENV], "false");
    assert_eq!(
        verifier.command.env[VERIFIER_SYFT_CACHE_ENV],
        VERIFIER_SYFT_CACHE_PATH
    );
    assert_eq!(
        verifier.command.program,
        "/usr/libexec/hephaestus/oci-verify"
    );
    assert_eq!(verifier.command.env[PLATFORM_OCI_VERIFIER_ENV], "1");
    assert!(verifier.command.args.is_empty());
    assert_eq!(
        verifier.labels["hephaestus.kind"],
        "repository_oci_verifier"
    );
    assert_eq!(
        verifier.labels["hephaestus.oci-job-id"],
        request.job_id.to_string()
    );
    assert_eq!(verifier.mounts.len(), 2);
    assert!(verifier.mounts[0].read_only);
    assert!(!verifier.mounts[1].read_only);
    assert_eq!(verifier.mounts[0].host_path, candidate);
    assert_eq!(verifier.mounts[1].host_path, verification);
}
