use super::*;

pub fn cooking_oci_worker_config(
    root: &Path,
    repository_root: &Path,
    workload_phase_timing: bool,
) -> Option<OciBuilderWorkerConfig> {
    let builder_image = env::var("HEPHAESTUS_TEST_OCI_BUILDER_VM_IMAGE").ok()?;
    let verifier_image = env::var("HEPHAESTUS_TEST_OCI_VERIFIER_VM_IMAGE").ok()?;
    let base_manifest = PathBuf::from(env::var("HEPHAESTUS_TEST_OCI_BASE_LAYOUT_MANIFEST").ok()?);
    let rootfs_root = PathBuf::from(env::var("HEPHAESTUS_TEST_OCI_ROOTFS_ROOT").ok()?);
    let registry_service = env::var("HEPHAESTUS_TEST_REGISTRY_SERVICE").ok()?;
    let registry_origin = env::var("HEPHAESTUS_TEST_REGISTRY_ORIGIN").ok()?;
    let output_root = root.join("repository-images/candidates");
    let checkout_root = root.join("repository-images/checkouts");
    let verification_root = root.join("repository-images/verification");
    let scratch_root = root.join("repository-images/scratch");
    let credential_root = root.join("repository-images/registry-credentials");
    for path in [
        &output_root,
        &checkout_root,
        &verification_root,
        &scratch_root,
        &credential_root,
    ] {
        std::fs::create_dir_all(path).expect("create cooking OCI worker root");
        std::fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(0o700))
            .expect("set cooking OCI worker root mode");
    }
    let mut image_layouts: std::collections::BTreeMap<String, PathBuf> = serde_json::from_slice(
        &std::fs::read(base_manifest).expect("read cooking OCI base manifest"),
    )
    .expect("parse cooking OCI base manifest");
    // The Hugo image uses the reviewed Python base. Registry names may differ
    // between its local runtime reference and published layout, but the digest
    // must remain identical. A same-name image with different bytes is not an
    // alias and must never stand in for the declared build provenance.
    let python_reference = env::var("HEPHAESTUS_LIBKRUN_UBUNTU_IMAGE")
        .expect("cooking Python image catalog reference");
    let layout = cooking_base_layout(&image_layouts, &python_reference)
        .expect("reviewed Python base layout with the exact cooking image digest");
    image_layouts.insert(python_reference, layout);
    let authority =
        RegistryAuthority::parse(&registry_service).expect("cooking registry authority");
    let skopeo_binary = env::var_os("HEPHAESTUS_SKOPEO")
        .map(PathBuf::from)
        .expect("HEPHAESTUS_SKOPEO must identify the trusted host skopeo binary");
    let oras_binary = env::var_os("HEPHAESTUS_ORAS")
        .map(PathBuf::from)
        .expect("HEPHAESTUS_ORAS must identify the trusted host oras binary");
    let publisher = PublisherConfiguration::new(
        authority,
        &output_root,
        &verification_root,
        &credential_root,
        &skopeo_binary,
        &oras_binary,
    )
    .expect("configure cooking OCI publisher")
    .with_registry_origin(&registry_origin)
    .expect("configure cooking OCI registry origin");
    let guest_init = env::var_os("HEPHAESTUS_GUEST_INIT_BINARY").map_or_else(
        || {
            let target_dir = env::var_os("CARGO_TARGET_DIR")
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target"));
            target_dir.join("x86_64-unknown-linux-musl/release/heph-init")
        },
        PathBuf::from,
    );
    let config = OciBuilderWorkerConfig {
        runtime: LocalOciRuntimeConfig {
            repository_root: repository_root.to_path_buf(),
            checkout_root,
            image_layouts,
            output_root,
            verified_rootfs_root: Some(verification_root.clone()),
            git_binary: PathBuf::from("/usr/bin/git"),
            tar_binary: PathBuf::from("/usr/bin/tar"),
            buildah_binary: None,
            trivy_binary: None,
            umoci_binary: None,
            buildah_output_prefix: String::from("heph-cooking-builder"),
        },
        publisher,
        publication_policy_version: registry_domain::PolicyVersion::parse("cooking/v1")
            .expect("cooking OCI policy version"),
        publication_policy: SupplyChainPolicy::without_signature(),
        builder_vm_image: builder_catalog_domain::OciImageReference::parse(&builder_image)
            .expect("cooking builder image reference"),
        verifier_vm_image: builder_catalog_domain::OciImageReference::parse(&verifier_image)
            .expect("cooking verifier image reference"),
        verification_root,
        scratch_root,
        // Ubuntu packages mkfs.ext4 as a symlink to mke2fs. Resolve the
        // trusted system path before the runtime rejects symlinked executables.
        mkfs_ext4: std::fs::canonicalize("/usr/sbin/mkfs.ext4")
            .expect("canonical mkfs.ext4 must be installed"),
        vm_resources: vm_trait::VmResources {
            vcpus: 2,
            // Buildah plus the libkrun VMM needs the worker's 8 GiB cgroup
            // ceiling for its backing pages; keep the declared guest budget
            // at the reviewed 2 GiB operation allocation.
            memory_mib: 2048,
        },
        workload_phase_timing,
        preparation_worker_name: String::from("golden-cooking-oci-preparation"),
        materialization_worker_name: String::from("golden-cooking-oci-materialization"),
        rootfs_root,
        root_manifest: root.join("repository-builder-roots.json"),
        guest_init,
        lease: Duration::from_secs(900),
        poll_interval: Duration::from_millis(100),
    };
    Some(config)
}
