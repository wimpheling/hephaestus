//! Administrator-owned local runtime adapters for repository OCI images.
//!
//! The adapters deliberately accept only opaque durable job identities and
//! paths derived from configured roots. Repository text never becomes a host
//! path, command executable, registry credential, or scanner argument.

use async_trait::async_trait;
use builder_catalog_domain::{OciDigest, OciImageReference};
use oci_builder_worker::{
    ClaimedProductionJob, IsolatedOciBuild, OciBuildEngine, OciImageProductionOutput,
    OciOutputPublisher, OciRootfsExporter, OciWorkerError, PreparedSource,
    RegistryPublisherTokenIssuer, RepositoryOciImagePublicationLease,
    RepositoryOciImagePublicationStore, SourceCheckoutProvider, local_image_name,
};
use registry_domain::{
    OciDescriptor, OciMediaType, PublicationIntent, PublicationState, SupplyChainReferrerKind,
};
use registry_publisher::{
    CommandRunner, ControlledOciPublisher, PublicationEvidenceFiles, PublicationMaterial,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command as ProcessCommand, Stdio},
    sync::Arc,
    time::Duration,
};
use tokio::process::Command;
use vm_trait::{
    DiskFormat, GuestCommand, NetworkMode, RootFilesystem, VmDisk, VmEvent, VmId, VmMount,
    VmProvider, VmResources, VmSpec,
};

const TRUSTED_SYSTEM_PATH: &str = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";
const BUILDER_SOURCE_GUEST_PATH: &str = "/workspace/source";
const BUILDER_BASE_GUEST_PATH: &str = "/workspace/heph-base";
const BUILDER_OUTPUT_GUEST_PATH: &str = "/workspace/candidate";
const BUILDER_SCRATCH_GUEST_PATH: &str = "/workspace/buildah";
const BUILDER_SCRATCH_DISK_ID: &str = "repository-oci-scratch";
// Buildah can hold the imported base, the working container, and the exported
// layer at once. Keep the job-scoped ext4 disk sparse on the host, but give the
// guest enough capacity for that bounded copy-on-write peak.
const BUILDER_SCRATCH_BYTES: u64 = 8 * 1024 * 1024 * 1024;
const VERIFIER_OUTPUT_GUEST_PATH: &str = "/workspace/verification";
const PLATFORM_OCI_BUILDER_ENV: &str = "HEPH_PLATFORM_OCI_BUILDER";
const PLATFORM_OCI_VERIFIER_ENV: &str = "HEPH_PLATFORM_OCI_VERIFIER";
const VERIFIER_TRIVY_CACHE_ENV: &str = "TRIVY_CACHE_DIR";
const VERIFIER_TRIVY_CACHE_PATH: &str = "/workspace/verification/trivy-cache";
const VERIFIER_SYFT_UPDATE_ENV: &str = "SYFT_CHECK_FOR_APP_UPDATE";
const VERIFIER_SYFT_CACHE_ENV: &str = "XDG_CACHE_HOME";
const VERIFIER_SYFT_CACHE_PATH: &str = "/workspace/verification/syft-cache";

/// Fixed local roots and VM resources for repository OCI operations.
///
/// The builder and verifier roots are platform-operation images selected by
/// the local operator, never repository metadata.
#[derive(Debug, Clone)]
pub struct VmOciOperationConfig {
    /// Platform-owned root for the one-shot Buildah guest.
    pub builder_root: RootFilesystem,
    /// Platform-owned root for the independent verifier guest.
    pub verifier_root: RootFilesystem,
    /// Private root containing one candidate layout per durable preparation.
    pub candidate_root: PathBuf,
    /// Private root containing one transient Buildah store per preparation.
    pub scratch_root: PathBuf,
    /// Absolute administrator-owned `mkfs.ext4` executable.
    pub mkfs_ext4: PathBuf,
    /// Private root containing verifier evidence and exported roots.
    pub verification_root: PathBuf,
    /// Fixed bounded resources for each operational guest.
    pub resources: VmResources,
}

/// Verified local outputs produced by distinct builder and verifier VMs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedVmOciOutput {
    /// Sealed candidate OCI layout mounted read-only into the verifier.
    pub layout: PathBuf,
    /// Verifier-created SPDX SBOM.
    pub sbom: PathBuf,
    /// Verifier-created offline vulnerability report.
    pub scan: PathBuf,
    /// Verifier-exported root filesystem for later atomic materialization.
    pub rootfs: PathBuf,
    /// Exact OCI manifest digest observed by the verifier.
    pub manifest_digest: OciDigest,
}

#[derive(Debug)]
struct ScratchDisk {
    path: PathBuf,
    filesystem_uuid: uuid::Uuid,
}

/// Executes a repository OCI build and independent verification in fresh VMs.
///
/// Publication deliberately remains outside this adapter: neither VM receives
/// a registry token, and a caller must validate these outputs before issuing a
/// host-controlled publication credential.
#[derive(Clone)]
pub struct VmOciOperation {
    provider: Arc<dyn VmProvider>,
    builder_root: RootFilesystem,
    verifier_root: RootFilesystem,
    candidate_root: PathBuf,
    scratch_root: PathBuf,
    mkfs_ext4: PathBuf,
    verification_root: PathBuf,
    resources: VmResources,
}

impl VmOciOperation {
    /// Validates private roots and creates a reusable VM operation boundary.
    ///
    /// # Errors
    ///
    /// Returns an error for unsafe roots or zero operational resources.
    pub fn initialize(
        provider: Arc<dyn VmProvider>,
        mut config: VmOciOperationConfig,
    ) -> Result<Self, OciWorkerError> {
        if config.resources.vcpus == 0
            || config.resources.memory_mib == 0
            || !config.candidate_root.is_absolute()
            || !config.scratch_root.is_absolute()
            || !config.verification_root.is_absolute()
        {
            return Err(OciWorkerError::InvalidConfiguration);
        }
        config.candidate_root = initialize_private_directory(&config.candidate_root)?;
        config.scratch_root = initialize_private_directory(&config.scratch_root)?;
        validate_executable(&config.mkfs_ext4)?;
        config.mkfs_ext4 =
            fs::canonicalize(&config.mkfs_ext4).map_err(OciWorkerError::Filesystem)?;
        config.verification_root = initialize_private_directory(&config.verification_root)?;
        if roots_overlap(&config.candidate_root, &config.scratch_root)
            || roots_overlap(&config.candidate_root, &config.verification_root)
            || roots_overlap(&config.scratch_root, &config.verification_root)
        {
            return Err(OciWorkerError::InvalidConfiguration);
        }
        Ok(Self {
            provider,
            builder_root: config.builder_root,
            verifier_root: config.verifier_root,
            candidate_root: config.candidate_root,
            scratch_root: config.scratch_root,
            mkfs_ext4: config.mkfs_ext4,
            verification_root: config.verification_root,
            resources: config.resources,
        })
    }

    /// Builds and verifies one exact request using two separate VMs.
    ///
    /// # Errors
    ///
    /// Returns a safe worker error when either guest cannot prove a successful
    /// bounded operation or creates malformed output.
    pub async fn execute(
        &self,
        request: &IsolatedOciBuild,
    ) -> Result<VerifiedVmOciOutput, OciWorkerError> {
        if !request.network_disabled || !request.ambient_credentials_disabled {
            return Err(OciWorkerError::InvalidConfiguration);
        }
        let candidate = self.candidate_root.join(request.job_id.to_string());
        let scratch = prepare_scratch_disk(&self.scratch_root, &self.mkfs_ext4, request.job_id)?;
        let verification = self.verification_root.join(request.job_id.to_string());
        prepare_empty_directory(&candidate)?;
        prepare_empty_directory(&verification)?;
        let result = async {
            self.run_builder(request, &candidate, &scratch).await?;
            // The builder emits the standard OCI outer-index-to-manifest form.
            // Convert that untrusted output into the one platform index the
            // publisher accepts, before it becomes an immutable verifier input.
            if symlinked_tree(&candidate)? {
                return Err(OciWorkerError::InvalidOutput);
            }
            normalize_layout_to_single_index(&candidate)?;
            seal_candidate_layout(&candidate)?;
            self.run_verifier(request, &candidate, &verification)
                .await?;
            verified_vm_output(&candidate, &verification)
        }
        .await;
        if result.is_err() {
            let _ignored = remove_private_directory(&candidate);
            let _ignored = fs::remove_file(&scratch.path);
            let _ignored = remove_private_directory(&verification);
        } else {
            let _ignored = fs::remove_file(&scratch.path);
        }
        result
    }

    async fn run_builder(
        &self,
        request: &IsolatedOciBuild,
        candidate: &Path,
        scratch: &ScratchDisk,
    ) -> Result<(), OciWorkerError> {
        self.run_guest(
            "builder",
            builder_vm_spec(
                request,
                candidate,
                scratch,
                &self.builder_root,
                &self.resources,
            )?,
        )
        .await
    }

    async fn run_verifier(
        &self,
        request: &IsolatedOciBuild,
        candidate: &Path,
        verification: &Path,
    ) -> Result<(), OciWorkerError> {
        self.run_guest(
            "verifier",
            verifier_vm_spec(
                request,
                candidate,
                verification,
                &self.verifier_root,
                &self.resources,
            ),
        )
        .await
    }

    async fn run_guest(&self, phase: &'static str, spec: VmSpec) -> Result<(), OciWorkerError> {
        let instance =
            self.provider
                .provision(spec)
                .await
                .map_err(|_| OciWorkerError::IsolatedVmFailed {
                    phase,
                    exit_code: None,
                })?;
        let mut events = instance.subscribe_events();
        if instance.start().await.is_err() {
            let _ignored = instance.destroy().await;
            return Err(OciWorkerError::IsolatedVmFailed {
                phase: operation_phase(phase, "startup"),
                exit_code: None,
            });
        }
        let exit = instance.wait().await.ok();
        let completed = exit
            .as_ref()
            .is_some_and(|exit| exit.code == Some(0) && exit.signal.is_none());
        let failure_phase = if completed {
            None
        } else {
            // `wait` is satisfied as soon as the provider records the terminal
            // status. The independent guest-control task can still be
            // forwarding the final bounded log frames. Wait only until its
            // terminal event (or the short deadline), reduce those bytes to a
            // fixed safe phase, and discard them before any persistence.
            Some(guest_failure_phase(phase, &mut events).await)
        };
        let destroyed = instance.destroy().await.is_ok();
        if !destroyed || !completed {
            return Err(OciWorkerError::IsolatedVmFailed {
                phase: failure_phase.unwrap_or_else(|| operation_phase(phase, "execution")),
                exit_code: exit.and_then(|exit| exit.code),
            });
        }
        Ok(())
    }
}

fn operation_phase(operation: &'static str, stage: &'static str) -> &'static str {
    match (operation, stage) {
        ("builder", "startup") => "builder startup",
        ("builder", "execution") => "builder execution",
        ("verifier", "startup") => "verifier startup",
        ("verifier", "execution") => "verifier execution",
        _ => "operation execution",
    }
}

async fn guest_failure_phase(
    operation: &'static str,
    events: &mut tokio::sync::broadcast::Receiver<VmEvent>,
) -> &'static str {
    // Guest output can include repository-controlled Dockerfile data. Inspect
    // it only in memory for a fixed operational allowlist; never retain, log,
    // emit, or return the raw bytes.
    let mut tail = Vec::new();
    loop {
        let received = tokio::time::timeout(Duration::from_millis(250), events.recv()).await;
        match received {
            Ok(Ok(VmEvent::Log { bytes, .. })) => {
                let remaining = 16_384_usize.saturating_sub(tail.len());
                tail.extend(bytes.into_iter().take(remaining));
            }
            Ok(Ok(VmEvent::Exited(_)) | Err(_)) | Err(_) => break,
            Ok(Ok(_)) => {}
        }
    }
    classify_guest_failure(operation, &tail)
}

fn classify_guest_failure(operation: &'static str, output: &[u8]) -> &'static str {
    let lowered: Vec<_> = output.iter().map(u8::to_ascii_lowercase).collect();
    let contains = |needle: &[u8]| lowered.windows(needle.len()).any(|window| window == needle);
    if operation == "builder"
        && (contains(b"unshare") || contains(b"newuidmap") || contains(b"newgidmap"))
        && contains(b"operation not permitted")
    {
        "builder user-namespace setup"
    } else if operation == "builder" && contains(b"cannot set --network") {
        "builder network isolation contract"
    } else if operation == "builder" && contains(b"heph_oci_failure=base-import") {
        "builder approved-base import"
    } else if operation == "builder" && contains(b"heph_oci_failure=base-alias") {
        "builder approved-base alias"
    } else if operation == "builder" && contains(b"heph_oci_failure=dockerfile-build") {
        "builder Dockerfile build"
    } else if operation == "builder" && contains(b"heph_oci_failure=layout-export") {
        "builder layout export"
    } else if operation == "builder" && contains(b"heph-base") {
        "builder approved-base import"
    } else if operation == "verifier" && contains(b"heph_oci_failure=output-cleanup") {
        "verifier output cleanup"
    } else if operation == "verifier" && contains(b"heph_oci_failure=output-prepare") {
        "verifier output preparation"
    } else if operation == "verifier" && contains(b"heph_oci_failure=rootless-environment") {
        "verifier rootless environment"
    } else if operation == "verifier"
        && (contains(b"heph_oci_failure=cache-prepare") || contains(b"heph_oci_failure=cache-copy"))
    {
        "verifier offline scan cache"
    } else if operation == "verifier" && contains(b"heph_oci_failure=layout-validate") {
        "verifier OCI layout validation"
    } else if operation == "verifier" && contains(b"heph_oci_failure=sbom") {
        "verifier SBOM generation"
    } else if operation == "verifier" && contains(b"heph_oci_failure=vulnerability-scan") {
        "verifier vulnerability scan"
    } else if operation == "verifier" && contains(b"heph_oci_failure=rootfs-export") {
        "verifier rootfs export"
    } else if operation == "verifier" && contains(b"heph_oci_failure=rootfs-handoff") {
        "verifier rootfs handoff"
    } else if operation == "verifier" && contains(b"heph_oci_failure=bundle-cleanup") {
        "verifier bundle cleanup"
    } else if operation == "verifier"
        && (contains(b"heph_oci_failure=manifest-read")
            || contains(b"heph_oci_failure=manifest-write"))
    {
        "verifier manifest output"
    } else {
        operation_phase(operation, "execution")
    }
}

fn builder_vm_spec(
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

fn verifier_vm_spec(
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
        labels: operation_labels("repository_oci_verifier", request.job_id),
    }
}

fn operation_labels(kind: &str, job_id: uuid::Uuid) -> BTreeMap<String, String> {
    BTreeMap::from([
        (String::from("hephaestus.kind"), String::from(kind)),
        (String::from("hephaestus.oci-job-id"), job_id.to_string()),
    ])
}

fn mount(tag: &str, host_path: &Path, guest_path: &str, read_only: bool) -> VmMount {
    VmMount {
        tag: String::from(tag),
        host_path: host_path.to_path_buf(),
        guest_path: PathBuf::from(guest_path),
        read_only,
    }
}

fn roots_overlap(first: &Path, second: &Path) -> bool {
    first == second || first.starts_with(second) || second.starts_with(first)
}

fn prepare_scratch_disk(
    root: &Path,
    mkfs_ext4: &Path,
    job_id: uuid::Uuid,
) -> Result<ScratchDisk, OciWorkerError> {
    let path = root.join(format!("{job_id}.raw"));
    let file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(OciWorkerError::Filesystem)?;
    if let Err(error) = file.set_len(BUILDER_SCRATCH_BYTES) {
        let _ignored = fs::remove_file(&path);
        return Err(OciWorkerError::Filesystem(error));
    }
    let filesystem_uuid = uuid::Uuid::new_v4();
    let filesystem_uuid_text = filesystem_uuid.to_string();
    let status = ProcessCommand::new(mkfs_ext4)
        .args(["-q", "-F", "-U", &filesystem_uuid_text])
        .arg(&path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(OciWorkerError::Filesystem)?;
    if !status.success() {
        let _ignored = fs::remove_file(&path);
        return Err(OciWorkerError::InvalidConfiguration);
    }
    Ok(ScratchDisk {
        path,
        filesystem_uuid,
    })
}

fn initialize_private_directory(path: &Path) -> Result<PathBuf, OciWorkerError> {
    fs::create_dir_all(path).map_err(OciWorkerError::Filesystem)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(OciWorkerError::Filesystem)?;
    let path = canonical_directory(path)?;
    let mode = fs::metadata(&path)
        .map_err(OciWorkerError::Filesystem)?
        .permissions()
        .mode();
    (mode.trailing_zeros() >= 6)
        .then_some(path)
        .ok_or(OciWorkerError::InvalidConfiguration)
}

fn prepare_empty_directory(path: &Path) -> Result<(), OciWorkerError> {
    if path.exists() {
        return Err(OciWorkerError::UnsafeMaterializationPath);
    }
    fs::create_dir(path).map_err(OciWorkerError::Filesystem)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(OciWorkerError::Filesystem)
}

fn remove_private_directory(path: &Path) -> Result<(), OciWorkerError> {
    let metadata = fs::symlink_metadata(path).map_err(OciWorkerError::Filesystem)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(OciWorkerError::UnsafeMaterializationPath);
    }
    restore_directory_tree_owner_write(path)?;
    fs::remove_dir_all(path).map_err(OciWorkerError::Filesystem)
}

fn restore_directory_tree_owner_write(path: &Path) -> Result<(), OciWorkerError> {
    let metadata = fs::symlink_metadata(path).map_err(OciWorkerError::Filesystem)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(OciWorkerError::UnsafeMaterializationPath);
    }
    for entry in fs::read_dir(path).map_err(OciWorkerError::Filesystem)? {
        let entry = entry.map_err(OciWorkerError::Filesystem)?;
        if entry
            .file_type()
            .map_err(OciWorkerError::Filesystem)?
            .is_dir()
        {
            restore_directory_tree_owner_write(&entry.path())?;
        }
    }
    fs::set_permissions(
        path,
        fs::Permissions::from_mode(metadata.permissions().mode() | 0o700),
    )
    .map_err(OciWorkerError::Filesystem)
}

fn guest_child_path(
    root: &Path,
    child: &Path,
    allow_root: bool,
) -> Result<PathBuf, OciWorkerError> {
    let relative = child
        .strip_prefix(root)
        .map_err(|_| OciWorkerError::UnsafeSourcePath)?;
    if relative.as_os_str().is_empty() {
        return allow_root
            .then(|| PathBuf::from(BUILDER_SOURCE_GUEST_PATH))
            .ok_or(OciWorkerError::UnsafeSourcePath);
    }
    Ok(PathBuf::from(BUILDER_SOURCE_GUEST_PATH).join(relative))
}

fn seal_candidate_layout(path: &Path) -> Result<(), OciWorkerError> {
    let index = path.join("index.json");
    let layout = path.join("oci-layout");
    if !index.is_file() || !layout.is_file() || symlinked_tree(path)? {
        return Err(OciWorkerError::InvalidOutput);
    }
    readonly_tree(path)
}

fn symlinked_tree(path: &Path) -> Result<bool, OciWorkerError> {
    for entry in fs::read_dir(path).map_err(OciWorkerError::Filesystem)? {
        let entry = entry.map_err(OciWorkerError::Filesystem)?;
        let metadata = fs::symlink_metadata(entry.path()).map_err(OciWorkerError::Filesystem)?;
        if metadata.file_type().is_symlink()
            || (metadata.is_dir() && symlinked_tree(&entry.path())?)
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn readonly_tree(path: &Path) -> Result<(), OciWorkerError> {
    for entry in fs::read_dir(path).map_err(OciWorkerError::Filesystem)? {
        let entry = entry.map_err(OciWorkerError::Filesystem)?;
        let child = entry.path();
        let metadata = fs::symlink_metadata(&child).map_err(OciWorkerError::Filesystem)?;
        if metadata.is_dir() {
            readonly_tree(&child)?;
            fs::set_permissions(&child, fs::Permissions::from_mode(0o500))
                .map_err(OciWorkerError::Filesystem)?;
        } else {
            fs::set_permissions(&child, fs::Permissions::from_mode(0o400))
                .map_err(OciWorkerError::Filesystem)?;
        }
    }
    fs::set_permissions(path, fs::Permissions::from_mode(0o500)).map_err(OciWorkerError::Filesystem)
}

fn verified_vm_output(
    layout: &Path,
    verification: &Path,
) -> Result<VerifiedVmOciOutput, OciWorkerError> {
    let evidence = verification.join("evidence");
    let sbom = regular_output_file(&evidence, "sbom.spdx.json")?;
    let scan = regular_output_file(&evidence, "vulnerability-scan.json")?;
    let rootfs = verification.join("rootfs");
    let rootfs_metadata = fs::symlink_metadata(&rootfs).map_err(OciWorkerError::Filesystem)?;
    if rootfs_metadata.file_type().is_symlink() || !rootfs_metadata.is_dir() {
        return Err(OciWorkerError::InvalidOutput);
    }
    let digest_text = fs::read_to_string(regular_output_file(verification, "manifest-digest")?)
        .map_err(OciWorkerError::Filesystem)?;
    let manifest_digest = OciDigest::parse(digest_text.trim().to_owned())
        .map_err(|_| OciWorkerError::InvalidOutput)?;
    Ok(VerifiedVmOciOutput {
        layout: layout.to_path_buf(),
        sbom,
        scan,
        rootfs,
        manifest_digest,
    })
}

fn regular_output_file(parent: &Path, name: &str) -> Result<PathBuf, OciWorkerError> {
    let path = parent.join(name);
    let metadata = fs::symlink_metadata(&path).map_err(OciWorkerError::Filesystem)?;
    (!metadata.file_type().is_symlink() && metadata.is_file())
        .then_some(path)
        .ok_or(OciWorkerError::InvalidOutput)
}

fn copy_verified_rootfs(source: &Path, destination: &Path) -> Result<(), OciWorkerError> {
    let destination_metadata =
        fs::symlink_metadata(destination).map_err(OciWorkerError::Filesystem)?;
    if destination_metadata.file_type().is_symlink() || !destination_metadata.is_dir() {
        return Err(OciWorkerError::UnsafeMaterializationPath);
    }
    copy_tree_without_following_links(source, destination)
}

fn copy_tree_without_following_links(
    source: &Path,
    destination: &Path,
) -> Result<(), OciWorkerError> {
    for entry in fs::read_dir(source).map_err(OciWorkerError::Filesystem)? {
        let entry = entry.map_err(OciWorkerError::Filesystem)?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let metadata = fs::symlink_metadata(&source_path).map_err(OciWorkerError::Filesystem)?;
        let file_type = metadata.file_type();
        if file_type.is_dir() {
            fs::create_dir(&destination_path).map_err(OciWorkerError::Filesystem)?;
            fs::set_permissions(&destination_path, metadata.permissions())
                .map_err(OciWorkerError::Filesystem)?;
            copy_tree_without_following_links(&source_path, &destination_path)?;
        } else if file_type.is_file() {
            fs::copy(&source_path, &destination_path).map_err(OciWorkerError::Filesystem)?;
            fs::set_permissions(&destination_path, metadata.permissions())
                .map_err(OciWorkerError::Filesystem)?;
        } else if file_type.is_symlink() {
            let target = fs::read_link(&source_path).map_err(OciWorkerError::Filesystem)?;
            std::os::unix::fs::symlink(target, &destination_path)
                .map_err(OciWorkerError::Filesystem)?;
        } else {
            return Err(OciWorkerError::InvalidOutput);
        }
    }
    Ok(())
}

/// Explicit absolute binaries and private roots used by the local OCI worker.
#[derive(Debug, Clone)]
pub struct LocalOciRuntimeConfig {
    /// Root containing canonical bare repositories named `<uuid>.git`.
    pub repository_root: PathBuf,
    /// Private transient directory for exact source checkouts.
    pub checkout_root: PathBuf,
    /// Administrator-owned OCI layouts cached by immutable image reference.
    /// These are daemon cache entries, never repository-supplied paths.
    pub image_layouts: BTreeMap<String, PathBuf>,
    /// Private immutable OCI layout output directory.
    pub output_root: PathBuf,
    /// Optional private verifier output root. When configured, verified
    /// repository images are materialized only from a verifier-exported
    /// rootfs whose recorded digest matches the immutable image reference.
    pub verified_rootfs_root: Option<PathBuf>,
    /// Absolute trusted Git executable.
    pub git_binary: PathBuf,
    /// Absolute trusted Tar executable used only to unpack Git's exact archive.
    pub tar_binary: PathBuf,
    /// Optional legacy host Buildah executable. Production daemon composition
    /// leaves this unset and uses the isolated builder VM instead.
    pub buildah_binary: Option<PathBuf>,
    /// Optional legacy host Trivy executable. Production daemon composition
    /// leaves this unset and uses the independent verifier VM instead.
    pub trivy_binary: Option<PathBuf>,
    /// Optional trusted Umoci executable for administrator-cached base images.
    /// Repository image outputs use verifier-exported roots instead.
    pub umoci_binary: Option<PathBuf>,
    /// Administrator-owned local Buildah image-tag prefix.
    pub buildah_output_prefix: String,
}

/// Trusted tools used only after the networkless Buildah phase.
#[derive(Debug, Clone)]
pub struct ForgeZotPublicationConfig {
    /// Absolute trusted Syft executable used to produce SPDX SBOMs.
    pub syft_binary: PathBuf,
    /// Absolute, administrator-owned Syft configuration.
    pub syft_config: PathBuf,
}

impl ForgeZotPublicationConfig {
    /// Validates and canonicalizes trusted Syft tooling.
    ///
    /// # Errors
    ///
    /// Returns an error when either path is relative, missing, or symbolic.
    pub fn initialize(mut self) -> Result<Self, OciWorkerError> {
        validate_executable(&self.syft_binary)?;
        self.syft_binary =
            fs::canonicalize(&self.syft_binary).map_err(OciWorkerError::Filesystem)?;
        self.syft_config = canonical_regular_file(&self.syft_config)?;
        Ok(self)
    }
}

/// Local implementation of exact checkout, scan/publication, and rootfs export.
#[derive(Clone)]
pub struct LocalOciRuntime {
    config: Arc<LocalOciRuntimeConfig>,
}

impl LocalOciRuntime {
    /// Validates all administrator-controlled binaries and roots.
    ///
    /// # Errors
    ///
    /// Returns an error for a relative, symlinked, or missing configured path.
    pub fn initialize(mut config: LocalOciRuntimeConfig) -> Result<Self, OciWorkerError> {
        validate_executable(&config.git_binary)?;
        validate_executable(&config.tar_binary)?;
        if let Some(binary) = &config.buildah_binary {
            validate_executable(binary)?;
        }
        if let Some(binary) = &config.trivy_binary {
            validate_executable(binary)?;
        }
        if let Some(binary) = &config.umoci_binary {
            validate_executable(binary)?;
        }
        config.repository_root = canonical_directory(&config.repository_root)?;
        config.checkout_root = initialize_directory(&config.checkout_root)?;
        config.output_root = initialize_directory(&config.output_root)?;
        config.verified_rootfs_root = config
            .verified_rootfs_root
            .map(|path| initialize_directory(&path))
            .transpose()?;
        if config.checkout_root.starts_with(&config.repository_root)
            || config.output_root.starts_with(&config.repository_root)
            || config.checkout_root.starts_with(&config.output_root)
            || config.output_root.starts_with(&config.checkout_root)
            || config.verified_rootfs_root.as_ref().is_some_and(|root| {
                root == &config.repository_root
                    || root.starts_with(&config.repository_root)
                    || config.repository_root.starts_with(root)
                    || root == &config.checkout_root
                    || root.starts_with(&config.checkout_root)
                    || config.checkout_root.starts_with(root)
                    || root == &config.output_root
                    || root.starts_with(&config.output_root)
                    || config.output_root.starts_with(root)
            })
            || config.buildah_output_prefix.trim().is_empty()
            || config.buildah_output_prefix.len() > 160
        {
            return Err(OciWorkerError::InvalidConfiguration);
        }
        config.image_layouts = config
            .image_layouts
            .into_iter()
            .map(|(reference, path)| {
                OciImageReference::parse(reference.clone())
                    .map_err(|_| OciWorkerError::InvalidConfiguration)?;
                Ok((reference, canonical_directory(&path)?))
            })
            .collect::<Result<_, OciWorkerError>>()?;
        Ok(Self {
            config: Arc::new(config),
        })
    }

    /// Returns the exact local Buildah image name for a job's immutable image.
    #[must_use]
    pub fn buildah_image_name(&self, request: &IsolatedOciBuild) -> String {
        local_image_name(&self.config.buildah_output_prefix, request.image_id)
    }

    /// Returns the private output root used by sealed repository layouts.
    #[must_use]
    pub fn output_root(&self) -> &Path {
        &self.config.output_root
    }

    fn legacy_buildah(&self) -> Result<&Path, OciWorkerError> {
        self.config
            .buildah_binary
            .as_deref()
            .ok_or(OciWorkerError::InvalidConfiguration)
    }

    fn legacy_trivy(&self) -> Result<&Path, OciWorkerError> {
        self.config
            .trivy_binary
            .as_deref()
            .ok_or(OciWorkerError::InvalidConfiguration)
    }

    fn verified_rootfs_for(
        &self,
        image_reference: &OciImageReference,
    ) -> Result<Option<PathBuf>, OciWorkerError> {
        let Some(root) = &self.config.verified_rootfs_root else {
            return Ok(None);
        };
        let expected = image_reference
            .as_str()
            .rsplit_once('@')
            .map(|(_, digest)| digest)
            .ok_or(OciWorkerError::InvalidOutput)?;
        let mut matches = Vec::new();
        for entry in fs::read_dir(root).map_err(OciWorkerError::Filesystem)? {
            let entry = entry.map_err(OciWorkerError::Filesystem)?;
            let metadata =
                fs::symlink_metadata(entry.path()).map_err(OciWorkerError::Filesystem)?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(OciWorkerError::InvalidOutput);
            }
            let verification = entry.path();
            let manifest = verification.join("manifest-digest");
            let manifest_metadata = match fs::symlink_metadata(&manifest) {
                // A failed verifier leaves its job-scoped evidence root behind
                // for durable diagnostics. It never had a verified rootfs, so
                // it cannot poison a later materialization lookup.
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Ok(metadata) => metadata,
                Err(error) => return Err(OciWorkerError::Filesystem(error)),
            };
            if manifest_metadata.file_type().is_symlink() || !manifest_metadata.is_file() {
                return Err(OciWorkerError::InvalidOutput);
            }
            let digest = fs::read_to_string(&manifest).map_err(OciWorkerError::Filesystem)?;
            if digest.trim() != expected {
                continue;
            }
            let rootfs = verification.join("rootfs");
            let rootfs_metadata =
                fs::symlink_metadata(&rootfs).map_err(OciWorkerError::Filesystem)?;
            if rootfs_metadata.file_type().is_symlink() || !rootfs_metadata.is_dir() {
                return Err(OciWorkerError::InvalidOutput);
            }
            matches.push(rootfs);
        }
        match matches.len() {
            0 => Ok(None),
            1 => Ok(matches.pop()),
            _ => Err(OciWorkerError::InvalidOutput),
        }
    }

    async fn command_success(
        &self,
        binary: &Path,
        arguments: Vec<OsString>,
    ) -> Result<(), OciWorkerError> {
        let status = Command::new(binary)
            .env_clear()
            // OCI tools may invoke administrator-installed helpers. Keep the
            // path fixed instead of inheriting caller-controlled state.
            .env("PATH", TRUSTED_SYSTEM_PATH)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .args(&arguments)
            .status()
            .await
            .map_err(OciWorkerError::Process)?;
        status
            .success()
            .then_some(())
            .ok_or(OciWorkerError::BuildFailed)
    }

    fn layout_directory(&self, request: &IsolatedOciBuild) -> PathBuf {
        self.config
            .output_root
            .join(request.image_id.as_uuid().to_string())
    }

    async fn publish_layout(
        &self,
        request: &IsolatedOciBuild,
        image_name: &str,
    ) -> Result<PathBuf, OciWorkerError> {
        let layout = self.layout_directory(request);
        if layout.exists() {
            fs::remove_dir_all(&layout).map_err(OciWorkerError::Filesystem)?;
        }
        fs::create_dir_all(&layout).map_err(OciWorkerError::Filesystem)?;
        self.command_success(
            self.legacy_buildah()?,
            vec![
                OsString::from("push"),
                OsString::from("--format"),
                OsString::from("oci"),
                OsString::from(image_name),
                OsString::from(format!("oci:{}:latest", layout.display())),
            ],
        )
        .await?;
        normalize_layout_to_single_index(&layout)
    }

    async fn scan_layout(
        &self,
        request: &IsolatedOciBuild,
        image_name: &str,
        layout: &Path,
    ) -> Result<PathBuf, OciWorkerError> {
        let scan = layout.join("scan.json");
        let archive = self
            .config
            .output_root
            .join(format!("{}.oci.tar", request.image_id.as_uuid()));
        if archive.exists() {
            return Err(OciWorkerError::UnsafeSourcePath);
        }
        self.command_success(
            self.legacy_buildah()?,
            vec![
                OsString::from("push"),
                OsString::from("--format"),
                OsString::from("oci"),
                OsString::from(image_name),
                OsString::from(format!("oci-archive:{}:latest", archive.display())),
            ],
        )
        .await?;
        let scan_result = self
            .command_success(
                self.legacy_trivy()?,
                vec![
                    OsString::from("image"),
                    OsString::from("--input"),
                    archive.clone().into_os_string(),
                    OsString::from("--offline-scan"),
                    OsString::from("--exit-code"),
                    OsString::from("1"),
                    OsString::from("--severity"),
                    OsString::from("CRITICAL,HIGH"),
                    OsString::from("--format"),
                    OsString::from("json"),
                    OsString::from("--output"),
                    scan.as_os_str().to_os_string(),
                ],
            )
            .await;
        fs::remove_file(&archive).map_err(OciWorkerError::Filesystem)?;
        scan_result?;
        if !scan.is_file()
            || fs::symlink_metadata(&scan)
                .map_err(OciWorkerError::Filesystem)?
                .file_type()
                .is_symlink()
        {
            return Err(OciWorkerError::InvalidOutput);
        }
        Ok(scan)
    }

    async fn sbom_layout(
        &self,
        layout: &Path,
        tooling: &ForgeZotPublicationConfig,
    ) -> Result<PathBuf, OciWorkerError> {
        let sbom = layout.join("sbom.spdx.json");
        self.command_success(
            &tooling.syft_binary,
            vec![
                OsString::from("scan"),
                OsString::from(format!("oci-dir:{}", layout.display())),
                OsString::from("--config"),
                tooling.syft_config.as_os_str().to_owned(),
                OsString::from("--output"),
                OsString::from(format!("spdx-json={}", sbom.display())),
                OsString::from("--quiet"),
            ],
        )
        .await?;
        trusted_output_file(layout, &sbom)
    }

    async fn publication_material(
        &self,
        request: &IsolatedOciBuild,
        tooling: &ForgeZotPublicationConfig,
    ) -> Result<PreparedPublicationMaterial, OciWorkerError> {
        let image_name = self.buildah_image_name(request);
        let layout = self.publish_layout(request, &image_name).await?;
        let expected_manifest = layout_index_descriptor(&layout)?;
        let scan = self.scan_layout(request, &image_name, &layout).await?;
        let sbom = self.sbom_layout(&layout, tooling).await?;
        let provenance = layout.join("provenance.json");
        let provenance_document = ProvenanceDocument {
            version: 1,
            project_id: request.project_id,
            image_id: request.image_id.as_uuid(),
            base_reference: request.base_reference.as_str(),
            manifest_digest: expected_manifest.digest().as_str(),
        };
        fs::write(
            &provenance,
            serde_json::to_vec(&provenance_document).map_err(OciWorkerError::Serialization)?,
        )
        .map_err(OciWorkerError::Filesystem)?;
        let provenance = trusted_output_file(&layout, &provenance)?;
        Ok(PreparedPublicationMaterial {
            expected_manifest,
            material: PublicationMaterial {
                layout: layout.clone(),
                evidence: PublicationEvidenceFiles {
                    sbom,
                    provenance,
                    scan,
                    signature: None,
                },
            },
            layout,
        })
    }
}

#[async_trait]
impl SourceCheckoutProvider for LocalOciRuntime {
    async fn checkout(&self, job: &ClaimedProductionJob) -> Result<PreparedSource, OciWorkerError> {
        let repository = self
            .config
            .repository_root
            .join(format!("{}.git", job.repository_id));
        let repository = canonical_directory(&repository)?;
        if repository.parent() != Some(self.config.repository_root.as_path()) {
            return Err(OciWorkerError::UnsafeSourcePath);
        }
        // A worker may have died after materializing this durable job's source
        // but before its cleanup ran. Reclaim only the exact, direct child
        // owned by that job; never follow an unexpected link or remove an
        // arbitrary path left beneath the private checkout root.
        let checkout = prepare_job_checkout(&self.config.checkout_root, job.id)?;
        fs::create_dir(&checkout).map_err(OciWorkerError::Filesystem)?;
        let archive = checkout.join("source.tar");
        let archive_status = Command::new(&self.config.git_binary)
            .env_clear()
            .env("PATH", TRUSTED_SYSTEM_PATH)
            .stdin(Stdio::null())
            .stdout(Stdio::from(
                fs::File::create(&archive).map_err(OciWorkerError::Filesystem)?,
            ))
            .stderr(Stdio::null())
            .arg("--git-dir")
            .arg(&repository)
            .arg("archive")
            .arg("--format=tar")
            .arg(&job.source_revision)
            .status()
            .await
            .map_err(OciWorkerError::Process)?;
        if !archive_status.success() {
            let _ignored = fs::remove_dir_all(&checkout);
            return Err(OciWorkerError::BuildFailed);
        }
        let source = checkout.join("source");
        fs::create_dir(&source).map_err(OciWorkerError::Filesystem)?;
        let tar_status = Command::new(&self.config.tar_binary)
            .env_clear()
            .env("PATH", TRUSTED_SYSTEM_PATH)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .arg("--no-same-owner")
            .arg("--no-same-permissions")
            .arg("--extract")
            .arg("--file")
            .arg(&archive)
            .arg("--directory")
            .arg(&source)
            .status()
            .await
            .map_err(OciWorkerError::Process)?;
        let _ignored = fs::remove_file(&archive);
        if !tar_status.success() {
            let _ignored = fs::remove_dir_all(&checkout);
            return Err(OciWorkerError::BuildFailed);
        }
        let source = canonical_directory(&source)?;
        let base_oci_layout = self
            .config
            .image_layouts
            .get(job.base_reference.as_str())
            .cloned()
            .ok_or(OciWorkerError::InvalidConfiguration)?;
        Ok(PreparedSource {
            checkout_root: source,
            base_oci_layout,
        })
    }

    async fn cleanup(&self, source: &PreparedSource) -> Result<(), OciWorkerError> {
        let checkout = source
            .checkout_root
            .parent()
            .ok_or(OciWorkerError::UnsafeSourcePath)?;
        let checkout = fs::canonicalize(checkout).map_err(OciWorkerError::Filesystem)?;
        if checkout.parent() != Some(self.config.checkout_root.as_path()) {
            return Err(OciWorkerError::UnsafeSourcePath);
        }
        fs::remove_dir_all(checkout).map_err(OciWorkerError::Filesystem)
    }
}

/// Local OCI artifacts ready for the forge-controlled Zot publication step.
/// Verified files ready for the credentialed host publication boundary.
pub struct PreparedPublicationMaterial {
    /// Canonical descriptor read from the sealed OCI layout.
    pub expected_manifest: OciDescriptor,
    /// Image layout and bounded verifier evidence for publication.
    pub material: PublicationMaterial,
    /// Layout retained until rootfs materialization has completed.
    pub layout: PathBuf,
}

/// Combines local output processing with the forge's durable Zot publication protocol.
///
/// Buildah receives no registry token: the token is
/// acquired only after the local OCI index, scan, SBOM, and provenance exist.
pub struct ForgeZotOciPublisher<S, T, R> {
    runtime: LocalOciRuntime,
    legacy_tooling: Option<ForgeZotPublicationConfig>,
    publications: S,
    tokens: T,
    publisher: ControlledOciPublisher<R>,
}

/// Complete repository-image engine using separate builder and verifier VMs.
///
/// The embedded publisher is the only credentialed component. The VMs operate
/// entirely before it, with neither token nor registry client mounted.
pub struct VmPublishedOciEngine<S, T, R> {
    operation: VmOciOperation,
    publisher: ForgeZotOciPublisher<S, T, R>,
}

impl<S, T, R> VmPublishedOciEngine<S, T, R> {
    /// Composes VM operations with the host-controlled publisher.
    #[must_use]
    pub const fn new(operation: VmOciOperation, publisher: ForgeZotOciPublisher<S, T, R>) -> Self {
        Self {
            operation,
            publisher,
        }
    }
}

#[async_trait]
impl<S, T, R> OciBuildEngine for VmPublishedOciEngine<S, T, R>
where
    S: RepositoryOciImagePublicationStore,
    T: RegistryPublisherTokenIssuer,
    R: CommandRunner + Send + Sync + 'static,
{
    async fn build(
        &self,
        request: IsolatedOciBuild,
    ) -> Result<OciImageProductionOutput, OciWorkerError> {
        let output = self.operation.execute(&request).await?;
        let prepared = verified_vm_publication_material(&request, &output)?;
        self.publisher.publish_verified(&request, prepared).await
    }
}

impl<S, T, R> ForgeZotOciPublisher<S, T, R> {
    /// Creates the repository-image Zot publication adapter.
    #[must_use]
    pub const fn new(
        runtime: LocalOciRuntime,
        legacy_tooling: Option<ForgeZotPublicationConfig>,
        publications: S,
        tokens: T,
        publisher: ControlledOciPublisher<R>,
    ) -> Self {
        Self {
            runtime,
            legacy_tooling,
            publications,
            tokens,
            publisher,
        }
    }

    /// Publishes already independently verified OCI material.
    ///
    /// # Errors
    ///
    /// Returns a worker error if token issuance, registry publication, or
    /// registry read-back cannot prove the exact verified descriptor.
    pub async fn publish_verified(
        &self,
        request: &IsolatedOciBuild,
        prepared: PreparedPublicationMaterial,
    ) -> Result<OciImageProductionOutput, OciWorkerError>
    where
        S: RepositoryOciImagePublicationStore,
        T: RegistryPublisherTokenIssuer,
        R: CommandRunner + Send + Sync + 'static,
    {
        let lease = self
            .publications
            .begin_repository_image_publication(
                request.project_id,
                request.image_id,
                prepared.expected_manifest.clone(),
            )
            .await?;
        let approved = match lease {
            RepositoryOciImagePublicationLease::Approved(intent) => intent,
            RepositoryOciImagePublicationLease::Publish(intent) => {
                let token = match self.tokens.issue_pull_push(&intent).await {
                    Ok(token) => token,
                    Err(error) => {
                        self.publications
                            .retry_repository_image_publication(intent.id())
                            .await?;
                        return Err(error);
                    }
                };
                // Skopeo and ORAS are synchronous trusted host tools. The
                // token is issued only here, after the separate verifier VM
                // has completed, and is never visible to either VM.
                let verified = tokio::task::block_in_place(|| {
                    self.publisher
                        .publish(&intent, &prepared.material, token.token())
                })
                .map_err(|_| OciWorkerError::RegistryPublication);
                let verification = match verified {
                    Ok(verification) => verification,
                    Err(error) => {
                        self.publications
                            .retry_repository_image_publication(intent.id())
                            .await?;
                        return Err(error);
                    }
                };
                self.publications
                    .record_verified_and_approve(intent.id(), verification)
                    .await?
            }
        };
        zot_confirmed_output(&approved, prepared.layout)
    }
}

#[async_trait]
impl<S, T, R> OciOutputPublisher for ForgeZotOciPublisher<S, T, R>
where
    S: RepositoryOciImagePublicationStore,
    T: RegistryPublisherTokenIssuer,
    R: CommandRunner + Send + Sync + 'static,
{
    async fn publish(
        &self,
        request: &IsolatedOciBuild,
    ) -> Result<OciImageProductionOutput, OciWorkerError> {
        let prepared = self
            .runtime
            .publication_material(
                request,
                self.legacy_tooling
                    .as_ref()
                    .ok_or(OciWorkerError::InvalidConfiguration)?,
            )
            .await?;
        self.publish_verified(request, prepared).await
    }
}

/// The local runtime cannot publish repository-image output by itself.
///
/// This compatibility implementation deliberately fails closed until the
/// daemon composes [`ForgeZotOciPublisher`] with durable registry and token
/// ports. It replaces the former fabricated registry-like output path.
#[async_trait]
impl OciOutputPublisher for LocalOciRuntime {
    async fn publish(
        &self,
        _request: &IsolatedOciBuild,
    ) -> Result<OciImageProductionOutput, OciWorkerError> {
        Err(OciWorkerError::RegistryPublication)
    }
}

#[async_trait]
impl OciRootfsExporter for LocalOciRuntime {
    async fn export_rootfs(
        &self,
        image_reference: &OciImageReference,
        destination: &Path,
    ) -> Result<(), OciWorkerError> {
        if let Some(rootfs) = self.verified_rootfs_for(image_reference)? {
            return copy_verified_rootfs(&rootfs, destination);
        }
        let layout = self
            .config
            .image_layouts
            .get(image_reference.as_str())
            .ok_or(OciWorkerError::ImageNotCached)?;
        let umoci = self
            .config
            .umoci_binary
            .as_deref()
            .ok_or(OciWorkerError::ImageNotCached)?;
        let bundle = destination.join(".umoci-bundle");
        self.command_success(
            umoci,
            vec![
                OsString::from("unpack"),
                OsString::from("--rootless"),
                OsString::from("--image"),
                OsString::from(format!("{}:latest", layout.display())),
                bundle.as_os_str().to_os_string(),
            ],
        )
        .await?;
        let rootfs = bundle.join("rootfs");
        let entries = fs::read_dir(&rootfs).map_err(OciWorkerError::Filesystem)?;
        for entry in entries {
            let entry = entry.map_err(OciWorkerError::Filesystem)?;
            fs::rename(entry.path(), destination.join(entry.file_name()))
                .map_err(OciWorkerError::Filesystem)?;
        }
        fs::remove_dir_all(bundle).map_err(OciWorkerError::Filesystem)
    }
}

#[derive(Serialize)]
struct ProvenanceDocument<'a> {
    version: u32,
    project_id: uuid::Uuid,
    image_id: uuid::Uuid,
    base_reference: &'a str,
    manifest_digest: &'a str,
}

/// Binds independently verified VM output to the exact publication request.
///
/// # Errors
///
/// Returns an error when the sealed layout descriptor does not match the
/// verifier's observed digest or evidence cannot be safely retained.
pub fn verified_vm_publication_material(
    request: &IsolatedOciBuild,
    output: &VerifiedVmOciOutput,
) -> Result<PreparedPublicationMaterial, OciWorkerError> {
    let expected_manifest = layout_index_descriptor(&output.layout)?;
    if expected_manifest.digest().as_str() != output.manifest_digest.as_str() {
        return Err(OciWorkerError::InvalidOutput);
    }
    let evidence_root = output.sbom.parent().ok_or(OciWorkerError::InvalidOutput)?;
    let scan = trusted_output_file(evidence_root, &output.scan)?;
    let sbom = trusted_output_file(evidence_root, &output.sbom)?;
    let provenance = evidence_root.join("provenance.json");
    if provenance.exists() {
        return Err(OciWorkerError::InvalidOutput);
    }
    let document = ProvenanceDocument {
        version: 1,
        project_id: request.project_id,
        image_id: request.image_id.as_uuid(),
        base_reference: request.base_reference.as_str(),
        manifest_digest: output.manifest_digest.as_str(),
    };
    fs::write(
        &provenance,
        serde_json::to_vec(&document).map_err(OciWorkerError::Serialization)?,
    )
    .map_err(OciWorkerError::Filesystem)?;
    let provenance = trusted_output_file(evidence_root, &provenance)?;
    Ok(PreparedPublicationMaterial {
        expected_manifest,
        material: PublicationMaterial {
            layout: output.layout.clone(),
            evidence: PublicationEvidenceFiles {
                sbom,
                provenance,
                scan,
                signature: None,
            },
        },
        layout: output.layout.clone(),
    })
}

fn validate_executable(path: &Path) -> Result<(), OciWorkerError> {
    let metadata = fs::symlink_metadata(path).map_err(OciWorkerError::Filesystem)?;
    if !path.is_absolute()
        || metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.permissions().mode() & 0o111 == 0
    {
        return Err(OciWorkerError::InvalidConfiguration);
    }
    Ok(())
}

fn canonical_regular_file(path: &Path) -> Result<PathBuf, OciWorkerError> {
    if !path.is_absolute() {
        return Err(OciWorkerError::InvalidConfiguration);
    }
    let metadata = fs::symlink_metadata(path).map_err(OciWorkerError::Filesystem)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(OciWorkerError::InvalidConfiguration);
    }
    fs::canonicalize(path).map_err(OciWorkerError::Filesystem)
}

fn initialize_directory(path: &Path) -> Result<PathBuf, OciWorkerError> {
    if !path.is_absolute() {
        return Err(OciWorkerError::InvalidConfiguration);
    }
    fs::create_dir_all(path).map_err(OciWorkerError::Filesystem)?;
    canonical_directory(path)
}

fn canonical_directory(path: &Path) -> Result<PathBuf, OciWorkerError> {
    let path = fs::canonicalize(path).map_err(OciWorkerError::Filesystem)?;
    let metadata = fs::symlink_metadata(&path).map_err(OciWorkerError::Filesystem)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(OciWorkerError::InvalidConfiguration);
    }
    Ok(path)
}

fn prepare_job_checkout(root: &Path, job_id: uuid::Uuid) -> Result<PathBuf, OciWorkerError> {
    let checkout = root.join(job_id.to_string());
    let metadata = match fs::symlink_metadata(&checkout) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(checkout),
        Err(error) => return Err(OciWorkerError::Filesystem(error)),
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(OciWorkerError::UnsafeSourcePath);
    }
    let canonical = fs::canonicalize(&checkout).map_err(OciWorkerError::Filesystem)?;
    if canonical.parent() != Some(root) {
        return Err(OciWorkerError::UnsafeSourcePath);
    }
    fs::remove_dir_all(canonical).map_err(OciWorkerError::Filesystem)?;
    Ok(checkout)
}

fn normalize_layout_to_single_index(layout: &Path) -> Result<PathBuf, OciWorkerError> {
    let index_path = layout.join("index.json");
    let source: serde_json::Value =
        serde_json::from_slice(&fs::read(&index_path).map_err(OciWorkerError::Filesystem)?)
            .map_err(OciWorkerError::Serialization)?;
    let manifests = source
        .get("manifests")
        .and_then(serde_json::Value::as_array)
        .filter(|manifests| manifests.len() == 1)
        .ok_or(OciWorkerError::InvalidOutput)?;
    let mut manifest = manifests[0].clone();
    let digest = manifest
        .get("digest")
        .and_then(serde_json::Value::as_str)
        .ok_or(OciWorkerError::InvalidOutput)?;
    let digest = OciDigest::parse(digest.to_owned()).map_err(|_| OciWorkerError::InvalidOutput)?;
    let size = manifest
        .get("size")
        .and_then(serde_json::Value::as_u64)
        .filter(|size| *size > 0)
        .ok_or(OciWorkerError::InvalidOutput)?;
    manifest
        .get("mediaType")
        .and_then(serde_json::Value::as_str)
        .filter(|media_type| *media_type == OciMediaType::IMAGE_MANIFEST)
        .ok_or(OciWorkerError::InvalidOutput)?;
    let blob = layout.join("blobs").join("sha256").join(
        digest
            .as_str()
            .strip_prefix("sha256:")
            .ok_or(OciWorkerError::InvalidOutput)?,
    );
    let blob_bytes = fs::read(&blob).map_err(OciWorkerError::Filesystem)?;
    if u64::try_from(blob_bytes.len()).map_err(|_| OciWorkerError::InvalidOutput)? != size
        || format!("sha256:{:x}", Sha256::digest(&blob_bytes)) != digest.as_str()
    {
        return Err(OciWorkerError::InvalidOutput);
    }
    let object = manifest
        .as_object_mut()
        .ok_or(OciWorkerError::InvalidOutput)?;
    object.insert(
        String::from("platform"),
        serde_json::json!({ "os": "linux", "architecture": "amd64" }),
    );
    let index_bytes = serde_json::to_vec(&serde_json::json!({
        "schemaVersion": 2,
        "mediaType": OciMediaType::IMAGE_INDEX,
        "manifests": [manifest],
    }))
    .map_err(OciWorkerError::Serialization)?;
    let index_digest = format!("sha256:{:x}", Sha256::digest(&index_bytes));
    let index_blob = layout.join("blobs").join("sha256").join(
        index_digest
            .strip_prefix("sha256:")
            .ok_or(OciWorkerError::InvalidOutput)?,
    );
    fs::write(&index_blob, &index_bytes).map_err(OciWorkerError::Filesystem)?;
    let reference_tag = format!("heph-{}", index_digest.replace(':', "-"));
    fs::write(
        &index_path,
        serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 2,
            "manifests": [{
                "mediaType": OciMediaType::IMAGE_INDEX,
                "digest": index_digest,
                "size": index_bytes.len(),
                "annotations": { "org.opencontainers.image.ref.name": reference_tag },
            }],
        }))
        .map_err(OciWorkerError::Serialization)?,
    )
    .map_err(OciWorkerError::Filesystem)?;
    Ok(layout.to_path_buf())
}

fn layout_index_descriptor(layout: &Path) -> Result<OciDescriptor, OciWorkerError> {
    let index: serde_json::Value = serde_json::from_slice(
        &fs::read(layout.join("index.json")).map_err(OciWorkerError::Filesystem)?,
    )
    .map_err(OciWorkerError::Serialization)?;
    let descriptor = index
        .get("manifests")
        .and_then(serde_json::Value::as_array)
        .and_then(|manifests| manifests.first())
        .ok_or(OciWorkerError::InvalidOutput)?;
    let digest = descriptor
        .get("digest")
        .and_then(serde_json::Value::as_str)
        .ok_or(OciWorkerError::InvalidOutput)?;
    let size = descriptor
        .get("size")
        .and_then(serde_json::Value::as_u64)
        .ok_or(OciWorkerError::InvalidOutput)?;
    OciDescriptor::new(
        registry_domain::Sha256Digest::parse(digest.to_owned())
            .map_err(|_| OciWorkerError::InvalidOutput)?,
        size,
        OciMediaType::parse(OciMediaType::IMAGE_INDEX)
            .map_err(|_| OciWorkerError::InvalidOutput)?,
    )
    .map_err(|_| OciWorkerError::InvalidOutput)
}

fn trusted_output_file(root: &Path, file: &Path) -> Result<PathBuf, OciWorkerError> {
    let file = fs::canonicalize(file).map_err(OciWorkerError::Filesystem)?;
    let metadata = fs::symlink_metadata(&file).map_err(OciWorkerError::Filesystem)?;
    (file.starts_with(root) && metadata.is_file() && !metadata.file_type().is_symlink())
        .then_some(file)
        .ok_or(OciWorkerError::InvalidOutput)
}

fn zot_confirmed_output(
    intent: &PublicationIntent,
    local_oci_layout: PathBuf,
) -> Result<OciImageProductionOutput, OciWorkerError> {
    if intent.state() != PublicationState::Approved {
        return Err(OciWorkerError::RegistryPublication);
    }
    let reference = intent
        .approved_reference()
        .map_err(|_| OciWorkerError::RegistryPublication)?;
    let verification = intent
        .verification()
        .ok_or(OciWorkerError::RegistryPublication)?;
    let evidence_reference = |kind| -> Result<String, OciWorkerError> {
        let referrer = verification
            .evidence()
            .referrers()
            .iter()
            .find(|referrer| referrer.kind() == kind)
            .ok_or(OciWorkerError::RegistryPublication)?;
        Ok(format!(
            "{}/{}@{}",
            reference.authority(),
            reference.namespace(),
            referrer.descriptor().digest()
        ))
    };
    let image_reference = OciImageReference::parse(reference.to_string())
        .map_err(|_| OciWorkerError::RegistryPublication)?;
    let image_digest = OciDigest::parse(reference.digest().as_str().to_owned())
        .map_err(|_| OciWorkerError::RegistryPublication)?;
    Ok(OciImageProductionOutput {
        image_reference,
        image_digest,
        attestation_reference: evidence_reference(SupplyChainReferrerKind::Provenance)?,
        sbom_reference: Some(evidence_reference(SupplyChainReferrerKind::Sbom)?),
        scan_reference: evidence_reference(SupplyChainReferrerKind::Scan)?,
        local_oci_layout,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        BUILDER_SCRATCH_DISK_ID, BUILDER_SCRATCH_GUEST_PATH, LocalOciRuntime,
        LocalOciRuntimeConfig, PLATFORM_OCI_BUILDER_ENV, PLATFORM_OCI_VERIFIER_ENV, ScratchDisk,
        VERIFIER_SYFT_CACHE_ENV, VERIFIER_SYFT_CACHE_PATH, VERIFIER_SYFT_UPDATE_ENV,
        VERIFIER_TRIVY_CACHE_ENV, VERIFIER_TRIVY_CACHE_PATH, builder_vm_spec,
        classify_guest_failure, copy_verified_rootfs, layout_index_descriptor,
        normalize_layout_to_single_index, prepare_job_checkout, remove_private_directory,
        verifier_vm_spec, zot_confirmed_output,
    };
    use builder_catalog_domain::{OciImageId, OciImageReference};
    use oci_builder_worker::{PreparedSource, SourceCheckoutProvider};
    use registry_domain::{
        ImmutableManifestReference, NamespaceClaim, OciDescriptor, OciMediaType,
        PlatformDescriptor, PolicyVersion, PublicationIntent, PublicationIntentId,
        RegistryAuthority, RegistryNamespace, Sha256Digest, SupplyChainEvidence, SupplyChainPolicy,
        SupplyChainReferrer, SupplyChainReferrerKind, VerifiedPublication,
    };
    use sha2::{Digest, Sha256};
    use std::{collections::BTreeMap, fs, os::unix::fs::PermissionsExt, path::PathBuf};
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
        fs::write(complete.join("manifest-digest"), format!("{digest}\n"))
            .expect("manifest digest");
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
            classify_guest_failure("verifier", b"operation not permitted"),
            "verifier execution"
        );
    }

    #[test]
    fn cleanup_removes_only_the_job_checkout() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let runtime = LocalOciRuntime::initialize(configuration(temporary.path()))
            .expect("safe local runtime");
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
            fs::set_permissions(directory, fs::Permissions::from_mode(0o500))
                .expect("seal directory");
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
}
