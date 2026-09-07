//! Test-only VM specification observation for the real provider path.
//!
//! The observer is deliberately a forwarding decorator rather than a fake
//! provider. It validates and summarizes the immutable specification before
//! handing the original value to libkrun. Host paths and command values are
//! never retained in the summaries.

use async_trait::async_trait;
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
};
use uuid::Uuid;
use vm_trait::{NetworkMode, RootFilesystem, VmError, VmId, VmInstance, VmProvider, VmSpec};

const AGENT_PATHS: [&str; 5] = [
    "/release",
    "/run/hephaestus",
    "/run/hephaestus-secrets",
    "/workspace/repo",
    "/workspace/work",
];
const GATEWAY_PATHS: [&str; 2] = ["/release", "/run/hephaestus"];
const BUILD_PATHS: [&str; 2] = ["/workspace/source", "/workspace/output"];

/// Redacted immutable VM specification captured at provider provision time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VmSpecSummary {
    /// Opaque VM identifier retained for exact test scoping.
    pub(crate) vm_id: String,
    /// Safe VM category derived from the `hephaestus.kind` labels.
    pub(crate) kind: String,
    /// Whether the selected root backing path existed as the required type.
    pub(crate) root_exists: bool,
    /// Read-only state disks and their opaque internal IDs.
    pub(crate) disks: Vec<DiskSummary>,
    /// Guest-visible mounts with known paths retained and unexpected paths hashed.
    pub(crate) mounts: Vec<MountSummary>,
    /// Exact provider-neutral network mode without host path data.
    pub(crate) network: NetworkSummary,
    /// Safe command classification and argument fingerprints.
    pub(crate) command: CommandSummary,
    /// Whether the provider received runtime authority for this VM.
    pub(crate) runtime_authority: bool,
}

/// Redacted disk information used by contract assertions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DiskSummary {
    /// Provider-defined disk identifier.
    pub(crate) id: String,
    /// Whether the caller requested read-only access.
    pub(crate) read_only: bool,
    /// Whether the caller-owned backing file existed at provision time.
    pub(crate) host_exists: bool,
}

/// Redacted mount information used by contract assertions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MountSummary {
    /// Stable mount tag fingerprint.
    pub(crate) tag_fingerprint: String,
    /// Known contract path, or a fingerprint for an unexpected path.
    pub(crate) guest_path: String,
    /// Whether the caller requested read-only access.
    pub(crate) read_only: bool,
    /// Whether the caller-owned backing directory existed at provision time.
    pub(crate) host_exists: bool,
}

/// Exact network mode captured without retaining provider-specific addresses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NetworkSummary {
    /// No guest IP networking.
    Disabled,
    /// Semantic broker transport only.
    BrokerOnly,
    /// User-mode networking and its bounded ingress count.
    UserMode { ingress_count: usize },
    /// A provider extension outside the reviewed cooking contract.
    Unknown,
}

/// Safe command metadata captured after sensitive values were checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CommandSummary {
    /// Coarse command class; the original program is discarded.
    pub(crate) class: CommandClass,
    /// Fingerprints of arguments, preserving no argument bytes.
    pub(crate) argument_fingerprints: Vec<String>,
    /// Environment names; environment values are discarded after scanning.
    pub(crate) environment_keys: Vec<String>,
}

/// Coarse command class used by the cooking contract checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CommandClass {
    /// A released application command under `/release`.
    Release,
    /// A trusted build bootstrap command.
    Build,
    /// Any command outside the known fixture contracts.
    Other,
}

/// Forwarding provider that records redacted specifications in memory.
pub(crate) struct VmSpecObserver {
    inner: Arc<dyn VmProvider>,
    patterns: Vec<Vec<u8>>,
    summaries: Mutex<Vec<VmSpecSummary>>,
}

impl VmSpecObserver {
    /// Wraps an already-constructed provider using caller-supplied fingerprints.
    pub(crate) fn new(
        inner: Arc<dyn VmProvider>,
        patterns: Vec<Vec<u8>>,
    ) -> Result<Arc<Self>, VmError> {
        if patterns.is_empty() || patterns.iter().any(Vec::is_empty) {
            return Err(observer_error(
                "observer requires nonempty fixture fingerprints",
            ));
        }
        Ok(Arc::new(Self {
            inner,
            patterns,
            summaries: Mutex::new(Vec::new()),
        }))
    }

    /// Returns a read-only snapshot of all observed specifications.
    pub(crate) fn summaries(&self) -> Vec<VmSpecSummary> {
        self.summaries.lock().expect("VM observer lock").clone()
    }

    /// Requires at least one observed VM for every requested safe category.
    pub(crate) fn assert_required_kinds(&self, required: &[&str]) -> Result<(), &'static str> {
        if required.is_empty() {
            return Err("VM observer requires an explicit VM kind set");
        }
        let summaries = self.summaries();
        for kind in required {
            if !summaries.iter().any(|summary| summary.kind == *kind) {
                return Err("VM observer did not observe every required VM kind");
            }
        }
        Ok(())
    }

    /// Checks the agent's complete cooking mount, authority, and network contract.
    pub(crate) fn assert_agent_contract(
        &self,
        expected_ids: &[String],
    ) -> Result<(), &'static str> {
        if expected_ids.is_empty() {
            return Err("VM observer requires explicit cooking agent IDs");
        }
        let summaries = self.summaries();
        let agents = selected_summaries(&summaries, expected_ids)?;
        for summary in agents {
            if !summary.root_exists
                || !summary.runtime_authority
                || summary.network != NetworkSummary::BrokerOnly
                || !has_exact_paths(&summary.mounts, &AGENT_PATHS)
                || !all_agent_mounts_are_read_only_except_work(&summary.mounts)
                || !mount_is_read_only(&summary.mounts, "/workspace/repo", true)
                || !mount_is_read_only(&summary.mounts, "/workspace/work", false)
                || summary.disks.len() != 1
                || !summary
                    .disks
                    .iter()
                    .all(|disk| disk.id == "instance-state" && !disk.read_only && disk.host_exists)
                || summary.command.class != CommandClass::Release
            {
                return Err("cooking agent VM contract is invalid");
            }
        }
        Ok(())
    }

    /// Checks the gateway's stateless, disabled-network launch contract.
    pub(crate) fn assert_gateway_contract(
        &self,
        expected_ids: &[String],
    ) -> Result<(), &'static str> {
        if expected_ids.is_empty() {
            return Err("VM observer requires explicit gateway IDs");
        }
        let summaries = self.summaries();
        let gateways = selected_summaries(&summaries, expected_ids)?;
        for summary in gateways {
            if !summary.root_exists
                || summary.network != NetworkSummary::Disabled
                || !summary.disks.is_empty()
                || !has_exact_paths(&summary.mounts, &GATEWAY_PATHS)
                || summary.mounts.iter().any(|mount| !mount.read_only)
                || !summary.runtime_authority
                || summary.command.class != CommandClass::Release
            {
                return Err("gateway VM contract is invalid");
            }
        }
        Ok(())
    }

    /// Checks the ordinary isolated cooking build launch contract.
    pub(crate) fn assert_build_contract(
        &self,
        expected_ids: &[String],
    ) -> Result<(), &'static str> {
        if expected_ids.is_empty() {
            return Err("VM observer requires explicit build IDs");
        }
        let summaries = self.summaries();
        let builds = selected_summaries(&summaries, expected_ids)?;
        for summary in builds {
            if !summary.root_exists
                || summary.network != NetworkSummary::Disabled
                || !summary.disks.is_empty()
                || !has_exact_paths(&summary.mounts, &BUILD_PATHS)
                || !mount_is_read_only(&summary.mounts, "/workspace/source", true)
                || !mount_is_read_only(&summary.mounts, "/workspace/output", false)
                || summary.runtime_authority
                || summary.command.class != CommandClass::Build
            {
                return Err("repository build VM contract is invalid");
            }
        }
        Ok(())
    }
}

fn selected_summaries<'a>(
    summaries: &'a [VmSpecSummary],
    expected_ids: &[String],
) -> Result<Vec<&'a VmSpecSummary>, &'static str> {
    let mut unique_ids = HashSet::with_capacity(expected_ids.len());
    let mut selected = Vec::with_capacity(expected_ids.len());
    for expected_id in expected_ids {
        if !unique_ids.insert(expected_id) {
            return Err("VM observer expected IDs must be unique");
        }
        let mut matches = summaries
            .iter()
            .filter(|summary| summary.vm_id == *expected_id);
        let Some(summary) = matches.next() else {
            return Err("VM observer did not observe every selected VM");
        };
        if matches.next().is_some() {
            return Err("VM observer observed a selected VM more than once");
        }
        selected.push(summary);
    }
    Ok(selected)
}

#[async_trait]
impl VmProvider for VmSpecObserver {
    fn name(&self) -> &'static str {
        self.inner.name()
    }

    async fn provision(&self, spec: VmSpec) -> Result<Arc<dyn VmInstance>, VmError> {
        let summary = summarize_spec(&spec, &self.patterns)?;
        let instance = self.inner.provision(spec).await?;
        self.summaries
            .lock()
            .map_err(|_| observer_error("observer state is unavailable"))?
            .push(summary);
        Ok(instance)
    }

    async fn cleanup_orphan(&self, id: &VmId) -> Result<(), VmError> {
        self.inner.cleanup_orphan(id).await
    }
}

fn summarize_spec(spec: &VmSpec, patterns: &[Vec<u8>]) -> Result<VmSpecSummary, VmError> {
    let vm_id = safe_vm_id(&spec.id, patterns)?;
    scan_value(spec.command.program.as_bytes(), patterns, "command")?;
    for argument in &spec.command.args {
        scan_value(argument.as_bytes(), patterns, "command argument")?;
    }
    for value in spec.command.env.values() {
        scan_value(value.as_bytes(), patterns, "command environment")?;
    }
    for key in spec.command.env.keys() {
        scan_value(key.as_bytes(), patterns, "command environment key")?;
    }

    let root_exists = match &spec.root {
        RootFilesystem::Directory { host_path } => host_path.is_dir(),
        RootFilesystem::Disk { host_path, .. } => host_path.is_file(),
        _ => false,
    };
    if !root_exists {
        return Err(observer_error("VM root backing resource is unavailable"));
    }
    let disks = spec
        .disks
        .iter()
        .map(|disk| DiskSummary {
            id: safe_disk_id(&disk.id),
            read_only: disk.read_only,
            host_exists: disk.host_path.is_file(),
        })
        .collect::<Vec<_>>();
    if disks.iter().any(|disk| !disk.host_exists) {
        return Err(observer_error("VM disk backing resource is unavailable"));
    }
    let mounts = spec
        .mounts
        .iter()
        .map(|mount| MountSummary {
            tag_fingerprint: fingerprint(mount.tag.as_bytes()),
            guest_path: safe_guest_path(&mount.guest_path),
            read_only: mount.read_only,
            host_exists: mount.host_path.is_dir(),
        })
        .collect::<Vec<_>>();
    if mounts.iter().any(|mount| !mount.host_exists) {
        return Err(observer_error("VM mount backing resource is unavailable"));
    }
    let mut environment_keys = spec.command.env.keys().cloned().collect::<Vec<_>>();
    environment_keys.sort();
    Ok(VmSpecSummary {
        vm_id,
        kind: safe_kind(spec),
        root_exists,
        disks,
        mounts,
        network: match &spec.network {
            NetworkMode::Disabled => NetworkSummary::Disabled,
            NetworkMode::BrokerOnly => NetworkSummary::BrokerOnly,
            NetworkMode::UserMode { ingress } => NetworkSummary::UserMode {
                ingress_count: ingress.len(),
            },
            _ => NetworkSummary::Unknown,
        },
        command: CommandSummary {
            class: command_class(&spec.command.program),
            argument_fingerprints: spec
                .command
                .args
                .iter()
                .map(|argument| fingerprint(argument.as_bytes()))
                .collect(),
            environment_keys,
        },
        runtime_authority: spec.runtime_authority.is_some(),
    })
}

fn safe_vm_id(id: &VmId, patterns: &[Vec<u8>]) -> Result<String, VmError> {
    scan_value(id.0.as_bytes(), patterns, "VM identifier")?;
    let is_uuid = Uuid::parse_str(&id.0).is_ok();
    let is_prefixed_uuid = ["agent-", "gateway-", "build-"].iter().any(|prefix| {
        id.0.strip_prefix(prefix)
            .is_some_and(|suffix| Uuid::parse_str(suffix).is_ok())
    });
    if is_uuid || is_prefixed_uuid {
        Ok(id.0.clone())
    } else {
        Ok(fingerprint(id.0.as_bytes()))
    }
}

fn scan_value(value: &[u8], patterns: &[Vec<u8>], field: &'static str) -> Result<(), VmError> {
    if patterns.iter().any(|pattern| {
        !pattern.is_empty() && value.windows(pattern.len()).any(|window| window == pattern)
    }) {
        return Err(observer_error(field));
    }
    Ok(())
}

fn observer_error(reason: &'static str) -> VmError {
    VmError::InvalidSpec {
        field: String::from("vm_observer"),
        reason: reason.to_owned(),
    }
}

fn safe_kind(spec: &VmSpec) -> String {
    match spec.labels.get("hephaestus.kind").map(String::as_str) {
        Some("gateway") => String::from("gateway"),
        Some("build") => String::from("build"),
        Some("repository_oci_builder") => String::from("repository_oci_builder"),
        Some("repository_oci_verifier") => String::from("repository_oci_verifier"),
        Some("agent") => String::from("agent"),
        None if spec.labels.contains_key("hephaestus.instance") => String::from("agent"),
        Some(_) | None => String::from("other"),
    }
}

fn safe_disk_id(id: &str) -> String {
    if id == "instance-state" {
        String::from(id)
    } else {
        fingerprint(id.as_bytes())
    }
}

fn fingerprint(value: &[u8]) -> String {
    let mut digest = Sha256::new();
    digest.update(value);
    format!("sha256:{:x}", digest.finalize())
}

fn safe_guest_path(path: &std::path::Path) -> String {
    let value = path.to_string_lossy();
    if AGENT_PATHS
        .iter()
        .chain(GATEWAY_PATHS.iter())
        .chain(BUILD_PATHS.iter())
        .any(|known| *known == value)
        || value == "/release-previous"
    {
        value.into_owned()
    } else {
        fingerprint(value.as_bytes())
    }
}

fn command_class(program: &str) -> CommandClass {
    if program.starts_with("/release/") {
        CommandClass::Release
    } else if program == "/bin/sh" || program == "/usr/bin/env" {
        CommandClass::Build
    } else {
        CommandClass::Other
    }
}

fn has_exact_paths(mounts: &[MountSummary], expected: &[&str]) -> bool {
    let paths: Vec<_> = mounts
        .iter()
        .map(|mount| mount.guest_path.as_str())
        .collect();
    expected.iter().all(|path| paths.contains(path)) && paths.len() == expected.len()
}

fn mount_is_read_only(mounts: &[MountSummary], path: &str, expected: bool) -> bool {
    mounts
        .iter()
        .find(|mount| mount.guest_path == path)
        .is_some_and(|mount| mount.read_only == expected)
}

fn all_agent_mounts_are_read_only_except_work(mounts: &[MountSummary]) -> bool {
    mounts.iter().all(|mount| {
        if mount.guest_path == "/workspace/work" {
            !mount.read_only
        } else {
            mount.read_only
        }
    })
}

#[cfg(test)]
mod tests {
    use super::{
        CommandClass, NetworkSummary, VmSpecObserver, VmSpecSummary, selected_summaries,
        summarize_spec,
    };
    use async_trait::async_trait;
    use std::{collections::BTreeMap, path::PathBuf, sync::Arc};
    use tempfile::TempDir;
    use vm_fake::FakeProvider;
    use vm_trait::{
        GuestCommand, NetworkMode, RootFilesystem, VmError, VmId, VmInstance, VmProvider,
        VmResources, VmSpec,
    };

    fn spec(root: &TempDir) -> VmSpec {
        VmSpec {
            id: VmId(String::from("00000000-0000-0000-0000-000000000001")),
            root: RootFilesystem::Directory {
                host_path: root.path().to_owned(),
            },
            disks: Vec::new(),
            mounts: Vec::new(),
            resources: VmResources {
                vcpus: 1,
                memory_mib: 64,
            },
            network: NetworkMode::Disabled,
            command: GuestCommand {
                program: String::from("/bin/sh"),
                args: vec![String::from("build.sh")],
                env: BTreeMap::new(),
                working_dir: Some(PathBuf::from("/")),
            },
            runtime_authority: None,
            labels: BTreeMap::from([(String::from("hephaestus.kind"), String::from("build"))]),
        }
    }

    #[test]
    fn requires_nonempty_fixture_patterns() {
        assert!(VmSpecObserver::new(Arc::new(FakeProvider::new()), Vec::new()).is_err());
        assert!(VmSpecObserver::new(Arc::new(FakeProvider::new()), vec![Vec::new()]).is_err());
    }

    #[tokio::test]
    async fn captures_safe_summary_and_forwards_to_real_provider() {
        let root = TempDir::new().expect("root");
        let observer =
            VmSpecObserver::new(Arc::new(FakeProvider::new()), vec![b"fixture".to_vec()])
                .expect("patterns");
        observer.provision(spec(&root)).await.expect("provision");
        let summaries = observer.summaries();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].kind, "build");
        assert_eq!(summaries[0].network, NetworkSummary::Disabled);
        assert_eq!(summaries[0].command.class, CommandClass::Build);
    }

    #[tokio::test]
    async fn rejects_missing_mount_without_disclosing_path() {
        let root = TempDir::new().expect("root");
        let observer =
            VmSpecObserver::new(Arc::new(FakeProvider::new()), vec![b"fixture".to_vec()])
                .expect("patterns");
        let mut invalid = spec(&root);
        invalid.mounts.push(vm_trait::VmMount {
            tag: String::from("bad"),
            host_path: root.path().join("missing-host-path"),
            guest_path: PathBuf::from("/unexpected/guest-path"),
            read_only: true,
        });
        let error = match observer.provision(invalid).await {
            Ok(_) => panic!("missing mount was accepted"),
            Err(error) => error,
        };
        assert!(!error.to_string().contains("missing-host-path"));
        assert!(observer.summaries().is_empty());
    }

    #[tokio::test]
    async fn rejects_network_contract_and_fixture_pattern_before_forwarding() {
        let root = TempDir::new().expect("root");
        let observer =
            VmSpecObserver::new(Arc::new(FakeProvider::new()), vec![b"fixture".to_vec()])
                .expect("patterns");
        let mut bad_network = spec(&root);
        bad_network.network = NetworkMode::UserMode {
            ingress: Vec::new(),
        };
        bad_network
            .labels
            .insert(String::from("fixture-label"), String::from("ok"));
        observer
            .provision(bad_network)
            .await
            .expect("observer records before assertion");
        assert_eq!(
            observer.summaries()[0].network,
            NetworkSummary::UserMode { ingress_count: 0 }
        );
        assert!(
            observer
                .assert_build_contract(&[String::from("00000000-0000-0000-0000-000000000001",)])
                .is_err()
        );

        let mut bad_value = spec(&root);
        bad_value.command.args = vec![String::from("fixture-value")];
        let error = match observer.provision(bad_value).await {
            Ok(_) => panic!("fixture pattern was accepted"),
            Err(error) => error,
        };
        assert!(!error.to_string().contains("fixture-value"));
    }

    struct FailingProvider;

    #[async_trait]
    impl VmProvider for FailingProvider {
        fn name(&self) -> &'static str {
            "failing"
        }
        async fn provision(&self, _: VmSpec) -> Result<Arc<dyn VmInstance>, VmError> {
            Err(VmError::Unavailable {
                resource: String::from("synthetic"),
                reason: String::from("forwarded"),
            })
        }
        async fn cleanup_orphan(&self, _: &VmId) -> Result<(), VmError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn preserves_forwarding_error() {
        let root = TempDir::new().expect("root");
        let observer = VmSpecObserver::new(Arc::new(FailingProvider), vec![b"fixture".to_vec()])
            .expect("patterns");
        let error = match observer.provision(spec(&root)).await {
            Ok(_) => panic!("forwarding error was lost"),
            Err(error) => error,
        };
        assert!(matches!(error, VmError::Unavailable { resource, .. } if resource == "synthetic"));
    }

    #[tokio::test]
    async fn rejects_extra_disk_and_writable_source_from_contract_summary() {
        let root = TempDir::new().expect("root");
        let observer =
            VmSpecObserver::new(Arc::new(FakeProvider::new()), vec![b"fixture".to_vec()])
                .expect("patterns");
        let mut invalid = spec(&root);
        invalid.mounts.push(vm_trait::VmMount {
            tag: String::from("source"),
            host_path: root.path().to_owned(),
            guest_path: PathBuf::from("/workspace/source"),
            read_only: false,
        });
        let disk = root.path().join("extra.raw");
        std::fs::write(&disk, b"disk").expect("disk");
        invalid.disks.push(vm_trait::VmDisk {
            id: String::from("foreign-disk"),
            host_path: disk,
            format: vm_trait::DiskFormat::Raw,
            read_only: false,
        });
        observer.provision(invalid).await.expect("record");
        assert!(
            observer
                .assert_build_contract(&[String::from("00000000-0000-0000-0000-000000000001",)])
                .is_err()
        );
    }

    fn selected_summary(root: &TempDir, id: &str) -> VmSpecSummary {
        let mut summary = summarize_spec(&spec(root), &[b"fixture".to_vec()])
            .expect("valid synthetic VM specification");
        summary.vm_id = id.to_owned();
        summary
    }

    #[test]
    fn selects_each_expected_id_once() {
        let root = TempDir::new().expect("root");
        let summaries = [selected_summary(&root, "a"), selected_summary(&root, "b")];
        let selected = selected_summaries(&summaries, &[String::from("a"), String::from("b")])
            .expect("each selected ID is observed exactly once");
        assert_eq!(selected.len(), 2);
    }

    #[test]
    fn rejects_duplicate_selected_id_when_another_id_is_missing() {
        let root = TempDir::new().expect("root");
        let summaries = [selected_summary(&root, "a"), selected_summary(&root, "a")];
        assert!(
            selected_summaries(&summaries, &[String::from("a"), String::from("b")],).is_err(),
            "a duplicate selected VM must not satisfy a missing selected ID"
        );
    }

    #[test]
    fn rejects_duplicate_expected_id() {
        let root = TempDir::new().expect("root");
        let summaries = [selected_summary(&root, "a")];
        assert!(selected_summaries(&summaries, &[String::from("a"), String::from("a")],).is_err());
    }

    #[tokio::test]
    async fn rejects_fixture_pattern_in_environment_key() {
        let root = TempDir::new().expect("root");
        let observer =
            VmSpecObserver::new(Arc::new(FakeProvider::new()), vec![b"fixture".to_vec()])
                .expect("patterns");
        let mut invalid = spec(&root);
        invalid
            .command
            .env
            .insert(String::from("fixture-key"), String::from("value"));
        let error = match observer.provision(invalid).await {
            Ok(_) => panic!("fixture environment key was accepted"),
            Err(error) => error,
        };
        assert!(!error.to_string().contains("fixture-key"));
    }

    #[tokio::test]
    async fn rejects_fixture_pattern_in_vm_identifier() {
        let root = TempDir::new().expect("root");
        let observer =
            VmSpecObserver::new(Arc::new(FakeProvider::new()), vec![b"fixture".to_vec()])
                .expect("patterns");
        let mut invalid = spec(&root);
        invalid.id = VmId(String::from("fixture-vm-id"));
        let error = match observer.provision(invalid).await {
            Ok(_) => panic!("fixture pattern in VM ID was accepted"),
            Err(error) => error,
        };
        assert!(!error.to_string().contains("fixture-vm-id"));
        assert!(observer.summaries().is_empty());
    }
}
