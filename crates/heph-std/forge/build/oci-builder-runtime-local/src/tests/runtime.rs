use crate::{
    config::{LocalOciRuntime, LocalOciRuntimeConfig},
    filesystem::{copy_verified_rootfs, remove_private_directory},
    guest::{classify_guest_failure, retain_log_tail},
    output::prepare_job_checkout,
    publication::zot_confirmed_output,
};
use builder_catalog_domain::{OciImageId, OciImageReference};
use oci_builder_worker::{PreparedSource, SourceCheckoutProvider};
use registry_domain::{
    ImmutableManifestReference, NamespaceClaim, OciDescriptor, OciMediaType, PlatformDescriptor,
    PolicyVersion, PublicationIntent, PublicationIntentId, RegistryAuthority, RegistryNamespace,
    Sha256Digest, SupplyChainEvidence, SupplyChainPolicy, SupplyChainReferrer,
    SupplyChainReferrerKind, VerifiedPublication,
};
use std::{collections::BTreeMap, fs, os::unix::fs::PermissionsExt};
use uuid::Uuid;

#[test]
fn verified_rootfs_copy_preserves_links_without_following_them() {
    let temporary = tempfile::tempdir().expect("temporary root");
    let source = temporary.path().join("source");
    let destination = temporary.path().join("destination");
    fs::create_dir_all(source.join("usr/bin")).expect("source tree");
    fs::create_dir(&destination).expect("destination tree");
    fs::write(source.join("usr/bin/tool"), "tool").expect("tool");
    std::os::unix::fs::symlink("usr/bin", source.join("bin")).expect("link");

    copy_verified_rootfs(&source, &destination).expect("copy verified root");

    assert_eq!(
        fs::read(destination.join("usr/bin/tool")).expect("copied tool"),
        b"tool"
    );
    assert_eq!(
        fs::read_link(destination.join("bin")).expect("copied link"),
        std::path::Path::new("usr/bin")
    );
}

#[test]
fn verified_rootfs_lookup_skips_incomplete_prior_verifier_attempts() {
    let temporary = tempfile::tempdir().expect("temporary root");
    let verification_root = temporary.path().join("verification");
    fs::create_dir(&verification_root).expect("verification root");
    fs::create_dir(verification_root.join("failed-attempt")).expect("failed attempt");
    let complete = verification_root.join("complete-attempt");
    fs::create_dir_all(complete.join("rootfs/usr/bin")).expect("verified rootfs");
    let digest = format!("sha256:{}", "a".repeat(64));
    fs::write(complete.join("manifest-digest"), format!("{digest}\n")).expect("manifest digest");
    fs::write(complete.join("rootfs/usr/bin/tool"), "tool").expect("rootfs tool");

    let mut config = configuration(temporary.path());
    config.verified_rootfs_root = Some(verification_root);
    let runtime = LocalOciRuntime::initialize(config).expect("safe local runtime");
    let reference = OciImageReference::parse(format!("registry.example/project@{digest}"))
        .expect("image reference");

    assert_eq!(
        runtime
            .verified_rootfs_for(&reference)
            .expect("verified rootfs lookup"),
        Some(complete.join("rootfs"))
    );
}

fn executable(root: &std::path::Path, name: &str) -> std::path::PathBuf {
    let path = root.join(name);
    fs::write(&path, "#!/bin/sh\nexit 0\n").expect("write test executable");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
        .expect("set test executable permissions");
    path
}

fn configuration(root: &std::path::Path) -> LocalOciRuntimeConfig {
    let base = root.join("base");
    fs::create_dir(&base).expect("create base layout directory");
    let binary = executable(root, "trusted-tool");
    LocalOciRuntimeConfig {
        repository_root: {
            let path = root.join("repositories");
            fs::create_dir(&path).expect("create repository root");
            path
        },
        checkout_root: root.join("checkouts"),
        image_layouts: BTreeMap::from([(
            String::from(
                "registry.example/heph-base@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            ),
            base,
        )]),
        output_root: root.join("outputs"),
        verified_rootfs_root: None,
        git_binary: binary.clone(),
        tar_binary: binary.clone(),
        buildah_binary: Some(binary.clone()),
        trivy_binary: Some(binary.clone()),
        umoci_binary: Some(binary),
        buildah_output_prefix: String::from("heph-image"),
    }
}

fn descriptor(character: char, media_type: &str) -> OciDescriptor {
    OciDescriptor::new(
        Sha256Digest::parse(format!("sha256:{}", character.to_string().repeat(64)))
            .expect("digest"),
        42,
        OciMediaType::parse(media_type).expect("media type"),
    )
    .expect("descriptor")
}

fn approved_intent() -> PublicationIntent {
    let project_id = Uuid::from_u128(1);
    let image_id = OciImageId::from_uuid(Uuid::from_u128(2));
    let namespace = RegistryNamespace::parse(format!(
        "projects/{project_id}/repository-images/{image_id}"
    ))
    .expect("repository image namespace");
    let claim = NamespaceClaim::new(namespace.owner().clone());
    let manifest = descriptor('a', OciMediaType::IMAGE_INDEX);
    let reference = ImmutableManifestReference::new(
        RegistryAuthority::parse("registry.example").expect("authority"),
        claim.namespace().clone(),
        manifest.digest().clone(),
    );
    let evidence = SupplyChainEvidence::new(
        manifest.digest().clone(),
        [
            (SupplyChainReferrerKind::Sbom, 'b'),
            (SupplyChainReferrerKind::Provenance, 'c'),
            (SupplyChainReferrerKind::Scan, 'd'),
        ]
        .into_iter()
        .map(|(kind, character)| {
            SupplyChainReferrer::new(
                kind,
                manifest.digest().clone(),
                descriptor(character, "application/vnd.oci.image.manifest.v1+json"),
                OciMediaType::parse(match kind {
                    SupplyChainReferrerKind::Sbom => "application/spdx+json",
                    SupplyChainReferrerKind::Provenance => "application/vnd.in-toto+json",
                    SupplyChainReferrerKind::Scan => {
                        "application/vnd.hephaestus.vulnerability-scan.v1+json"
                    }
                    SupplyChainReferrerKind::Signature => {
                        "application/vnd.dev.cosign.simplesigning.v1+json"
                    }
                })
                .expect("artifact type"),
            )
        })
        .collect(),
    )
    .expect("evidence");
    let verification = VerifiedPublication::new(
        &reference,
        manifest.clone(),
        vec![
            PlatformDescriptor::new(
                descriptor('e', OciMediaType::IMAGE_MANIFEST),
                "linux",
                "amd64",
                None,
            )
            .expect("platform"),
        ],
        evidence,
    )
    .expect("verification");
    PublicationIntent::new(
        PublicationIntentId::from_uuid(Uuid::from_u128(3)),
        claim,
        reference,
        manifest,
        PolicyVersion::parse("image-v1").expect("policy version"),
        SupplyChainPolicy::without_signature(),
    )
    .expect("intent")
    .begin_publishing()
    .expect("publishing")
    .record_verified(verification)
    .expect("verified")
    .approve()
    .expect("approved")
}

#[test]
fn initialization_rejects_overlapping_private_roots() {
    let temporary = tempfile::tempdir().expect("temporary directory");
    let mut config = configuration(temporary.path());
    config.output_root = config.checkout_root.clone();
    assert!(LocalOciRuntime::initialize(config).is_err());
}

#[test]
fn guest_failure_classification_redacts_raw_builder_output() {
    assert_eq!(
        classify_guest_failure(
            "builder",
            b"newuidmap: write to uid_map failed: Operation not permitted",
        ),
        "builder user-namespace setup"
    );
    assert_eq!(
        classify_guest_failure("builder", b"tenant-controlled diagnostic"),
        "builder execution"
    );
    assert_eq!(
        classify_guest_failure(
            "verifier",
            b"heph_oci_failure=rootfs-export tenant-controlled diagnostic",
        ),
        "verifier rootfs export"
    );
    assert_eq!(
        classify_guest_failure("builder", b"HEPH_OCI_FAILURE=base-import"),
        "builder approved-base import"
    );
    assert_eq!(
        classify_guest_failure("builder", b"STEP 1/3: FROM heph-base\nRUN build-hugo"),
        "builder execution"
    );
    assert_eq!(
        classify_guest_failure("verifier", b"operation not permitted"),
        "verifier execution"
    );
}

#[test]
fn bounded_log_tail_keeps_failure_markers_after_large_build_output() {
    let mut tail = Vec::new();
    retain_log_tail(&mut tail, &vec![b'x'; 20_000]);
    let marker = b"\nHEPH_OCI_FAILURE=dockerfile-build\n";
    retain_log_tail(&mut tail, &marker[..10]);
    retain_log_tail(&mut tail, &marker[10..]);
    assert_eq!(tail.len(), 16_384);
    assert_eq!(
        classify_guest_failure("builder", &tail),
        "builder Dockerfile build"
    );
    let mut oversized = vec![b'x'; 20_000];
    oversized.extend_from_slice(marker);
    retain_log_tail(&mut tail, &oversized);
    assert_eq!(tail.len(), 16_384);
    assert_eq!(
        classify_guest_failure("builder", &tail),
        "builder Dockerfile build"
    );
}

#[test]
fn cleanup_removes_only_the_job_checkout() {
    let temporary = tempfile::tempdir().expect("temporary directory");
    let runtime =
        LocalOciRuntime::initialize(configuration(temporary.path())).expect("safe local runtime");
    let checkout = temporary.path().join("checkouts/job/source");
    fs::create_dir_all(&checkout).expect("create source checkout");
    let source = PreparedSource {
        checkout_root: fs::canonicalize(&checkout).expect("canonical source checkout"),
        base_oci_layout: temporary.path().join("base"),
    };
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("Tokio runtime")
        .block_on(runtime.cleanup(&source))
        .expect("remove job checkout");
    assert!(!temporary.path().join("checkouts/job").exists());
    assert!(temporary.path().join("checkouts").exists());
}

#[test]
fn recovery_removes_only_an_existing_direct_job_checkout() {
    let temporary = tempfile::tempdir().expect("temporary directory");
    let root = temporary.path().join("checkouts");
    fs::create_dir(&root).expect("checkout root");
    let job = Uuid::from_u128(42);
    let stale = root.join(job.to_string());
    fs::create_dir_all(stale.join("source")).expect("stale job checkout");
    let unrelated = root.join("unrelated");
    fs::create_dir(&unrelated).expect("unrelated checkout");

    let checkout = prepare_job_checkout(&root, job).expect("safe stale checkout");

    assert_eq!(checkout, stale);
    assert!(!checkout.exists());
    assert!(unrelated.exists());
}

#[test]
fn recovery_rejects_a_symlinked_job_checkout() {
    let temporary = tempfile::tempdir().expect("temporary directory");
    let root = temporary.path().join("checkouts");
    let outside = temporary.path().join("outside");
    fs::create_dir(&root).expect("checkout root");
    fs::create_dir(&outside).expect("outside directory");
    let job = Uuid::from_u128(42);
    std::os::unix::fs::symlink(&outside, root.join(job.to_string())).expect("job symlink");

    assert!(matches!(
        prepare_job_checkout(&root, job),
        Err(oci_builder_worker::OciWorkerError::UnsafeSourcePath)
    ));
    assert!(outside.exists());
}

#[test]
fn cleanup_removes_a_sealed_private_directory_tree() {
    let temporary = tempfile::tempdir().expect("temporary directory");
    let sealed = temporary.path().join("sealed");
    let nested = sealed.join("blobs/sha256");
    fs::create_dir_all(&nested).expect("sealed tree");
    fs::write(nested.join("layer"), "layer").expect("sealed layer");
    for directory in [&sealed, &sealed.join("blobs"), &nested] {
        fs::set_permissions(directory, fs::Permissions::from_mode(0o500)).expect("seal directory");
    }

    remove_private_directory(&sealed).expect("remove sealed tree");

    assert!(!sealed.exists());
}

#[test]
fn only_a_verified_and_approved_zot_intent_becomes_durable_image_output() {
    let temporary = tempfile::tempdir().expect("temporary directory");
    let layout = temporary.path().join("layout");
    fs::create_dir(&layout).expect("layout");
    let output =
        zot_confirmed_output(&approved_intent(), layout.clone()).expect("Zot-confirmed output");
    assert_eq!(
        output.image_reference.as_str(),
        "registry.example/projects/00000000-0000-0000-0000-000000000001/repository-images/00000000-0000-0000-0000-000000000002@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    );
    assert!(output.attestation_reference.ends_with(&"c".repeat(64)));
    assert!(
        output
            .sbom_reference
            .expect("SBOM")
            .ends_with(&"b".repeat(64))
    );
    assert!(output.scan_reference.ends_with(&"d".repeat(64)));
    assert_eq!(output.local_oci_layout, layout);
}
