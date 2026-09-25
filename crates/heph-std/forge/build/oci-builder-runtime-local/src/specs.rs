//! Builder and verifier VM specifications.

use super::{
    constants::{
        BUILDER_BASE_GUEST_PATH, BUILDER_OUTPUT_GUEST_PATH, BUILDER_SCRATCH_DISK_ID,
        BUILDER_SCRATCH_GUEST_PATH, BUILDER_SOURCE_GUEST_PATH, PLATFORM_OCI_BUILDER_ENV,
        PLATFORM_OCI_VERIFIER_ENV, ScratchDisk, VERIFIER_OUTPUT_GUEST_PATH,
        VERIFIER_SYFT_CACHE_ENV, VERIFIER_SYFT_CACHE_PATH, VERIFIER_SYFT_UPDATE_ENV,
        VERIFIER_TRIVY_CACHE_ENV, VERIFIER_TRIVY_CACHE_PATH,
    },
    filesystem::guest_child_path,
};
use oci_builder_worker::{IsolatedOciBuild, OciWorkerError};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
use vm_trait::{
    DiskFormat, GuestCommand, NetworkMode, RootFilesystem, VmDisk, VmId, VmMount, VmResources,
    VmSpec,
};

pub fn builder_vm_spec(
    request: &IsolatedOciBuild,
    candidate: &Path,
    scratch: &ScratchDisk,
    builder_root: &RootFilesystem,
    resources: &VmResources,
) -> Result<VmSpec, OciWorkerError> {
    let dockerfile = guest_child_path(&request.checkout_root, &request.dockerfile, false)?;
    let context = guest_child_path(&request.checkout_root, &request.context, true)?;
    Ok(VmSpec {
        id: VmId(format!("oci-builder-{}", request.job_id)),
        root: builder_root.clone(),
        disks: vec![VmDisk {
            id: String::from(BUILDER_SCRATCH_DISK_ID),
            host_path: scratch.path.clone(),
            format: DiskFormat::Raw,
            read_only: false,
        }],
        mounts: vec![
            mount(
                "oci-source",
                &request.checkout_root,
                BUILDER_SOURCE_GUEST_PATH,
                true,
            ),
            mount(
                "oci-base",
                &request.base_oci_layout,
                BUILDER_BASE_GUEST_PATH,
                true,
            ),
            mount("oci-candidate", candidate, BUILDER_OUTPUT_GUEST_PATH, false),
        ],
        resources: resources.clone(),
        network: NetworkMode::Disabled,
        command: GuestCommand {
            program: String::from("/usr/libexec/hephaestus/oci-build"),
            args: vec![
                dockerfile.to_string_lossy().into_owned(),
                context.to_string_lossy().into_owned(),
            ],
            // Buildah must preserve ownership while importing an approved base
            // layer. Execute it as guest root only inside this dedicated,
            // networkless platform-operation VM; the flag never reaches the
            // repository-controlled build environment.
            env: BTreeMap::from([(String::from(PLATFORM_OCI_BUILDER_ENV), String::from("1"))]),
            working_dir: None,
        },
        runtime_authority: None,
        runtime_git_bridge: None,
        private_http_service: None,
        labels: BTreeMap::from([
            (
                String::from("hephaestus.kind"),
                String::from("repository_oci_builder"),
            ),
            (
                String::from("hephaestus.oci-job-id"),
                request.job_id.to_string(),
            ),
            (
                String::from("hephaestus.oci-scratch.filesystem-uuid"),
                scratch.filesystem_uuid.to_string(),
            ),
            (
                String::from("hephaestus.oci-scratch.mount-path"),
                String::from(BUILDER_SCRATCH_GUEST_PATH),
            ),
        ]),
    })
}

pub fn verifier_vm_spec(
    request: &IsolatedOciBuild,
    candidate: &Path,
    verification: &Path,
    verifier_root: &RootFilesystem,
    resources: &VmResources,
) -> VmSpec {
    VmSpec {
        id: VmId(format!("oci-verifier-{}", request.job_id)),
        root: verifier_root.clone(),
        disks: Vec::new(),
        mounts: vec![
            mount("oci-candidate", candidate, BUILDER_OUTPUT_GUEST_PATH, true),
            mount(
                "oci-verification",
                verification,
                VERIFIER_OUTPUT_GUEST_PATH,
                false,
            ),
        ],
        resources: resources.clone(),
        network: NetworkMode::Disabled,
        command: GuestCommand {
            program: String::from("/usr/libexec/hephaestus/oci-verify"),
            args: Vec::new(),
            // Guest startup deliberately constructs a minimal environment, so
            // platform-operation settings cannot rely on image `ENV` values.
            // The verifier command copies the pinned Trivy database into this
            // job-scoped writable cache, and Syft must not attempt an update
            // from a networkless guest.
            env: BTreeMap::from([
                // The root-only bootstrap recognizes the exact verifier
                // program plus this marker. It is a fixed platform operation,
                // never repository-controlled guest input.
                (String::from(PLATFORM_OCI_VERIFIER_ENV), String::from("1")),
                (
                    String::from(VERIFIER_TRIVY_CACHE_ENV),
                    String::from(VERIFIER_TRIVY_CACHE_PATH),
                ),
                (
                    String::from(VERIFIER_SYFT_UPDATE_ENV),
                    String::from("false"),
                ),
                (
                    String::from(VERIFIER_SYFT_CACHE_ENV),
                    String::from(VERIFIER_SYFT_CACHE_PATH),
                ),
            ]),
            working_dir: None,
        },
        runtime_authority: None,
        runtime_git_bridge: None,
        private_http_service: None,
        labels: operation_labels("repository_oci_verifier", request.job_id),
    }
}

pub fn operation_labels(kind: &str, job_id: uuid::Uuid) -> BTreeMap<String, String> {
    BTreeMap::from([
        (String::from("hephaestus.kind"), String::from(kind)),
        (String::from("hephaestus.oci-job-id"), job_id.to_string()),
    ])
}

pub fn mount(tag: &str, host_path: &Path, guest_path: &str, read_only: bool) -> VmMount {
    VmMount {
        tag: String::from(tag),
        host_path: host_path.to_path_buf(),
        guest_path: PathBuf::from(guest_path),
        read_only,
    }
}

pub fn roots_overlap(first: &Path, second: &Path) -> bool {
    first == second || first.starts_with(second) || second.starts_with(first)
}
