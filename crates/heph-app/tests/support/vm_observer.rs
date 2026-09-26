//! Test-only VM specification observation for the real provider path.
//!
//! The observer is deliberately a forwarding decorator rather than a fake
//! provider. It validates and summarizes the immutable specification before
//! handing the original value to libkrun. Host paths and command values are
//! never retained in the summaries.

use async_trait::async_trait;
use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
};
use vm_trait::{VmError, VmId, VmInstance, VmProvider, VmSpec};

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
        let summary = summary::summarize_spec(&spec, &self.patterns)?;
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

mod summary;
use summary::{
    all_agent_mounts_are_read_only_except_work, has_exact_paths, mount_is_read_only, observer_error,
};
