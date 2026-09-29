//! Provider-neutral registry inventory and retention reporting.
use crate::{PublicationIntent, PublicationState, RegistryNamespace, RegistryOwner, Sha256Digest};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Maximum number of manifest or referrer descriptors accepted in one
/// provider-neutral registry inventory snapshot.
///
/// Keeping this bounded makes the operator report safe to run against an
/// unavailable or unexpectedly large registry without retaining OCI bytes.
pub const MAX_REGISTRY_INVENTORY_ENTRIES: usize = 100_000;

/// A provider-neutral, descriptor-only observation from an OCI registry.
///
/// This intentionally contains neither credentials nor OCI content. Inventory
/// collectors for Zot or another distribution implementation can produce this
/// bounded value before the retention report is evaluated.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct RegistryInventoryEntry {
    repository_path: RegistryNamespace,
    digest: Sha256Digest,
}

impl RegistryInventoryEntry {
    /// Creates one canonical descriptor inventory entry.
    #[must_use]
    pub const fn new(repository_path: RegistryNamespace, digest: Sha256Digest) -> Self {
        Self {
            repository_path,
            digest,
        }
    }

    /// Returns the observed repository path.
    #[must_use]
    pub const fn repository_path(&self) -> &RegistryNamespace {
        &self.repository_path
    }

    /// Returns the observed descriptor digest.
    #[must_use]
    pub const fn digest(&self) -> &Sha256Digest {
        &self.digest
    }
}

/// Versioned bounded inventory input accepted by the operator report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistryInventoryDocument {
    /// Stable document schema revision.
    pub schema_version: u16,
    /// Manifest and referrer descriptors observed by an inventory provider.
    pub entries: Vec<RegistryInventoryEntry>,
}

/// A deduplicated bounded inventory snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryInventory {
    entries: BTreeSet<RegistryInventoryEntry>,
}

impl TryFrom<RegistryInventoryDocument> for RegistryInventory {
    type Error = RegistryRetentionReportError;

    fn try_from(document: RegistryInventoryDocument) -> Result<Self, Self::Error> {
        if document.schema_version != 1 {
            return Err(RegistryRetentionReportError::UnsupportedInventorySchema(
                document.schema_version,
            ));
        }
        if document.entries.len() > MAX_REGISTRY_INVENTORY_ENTRIES {
            return Err(RegistryRetentionReportError::InventoryTooLarge);
        }
        Ok(Self {
            entries: document.entries.into_iter().collect(),
        })
    }
}

impl RegistryInventory {
    /// Returns the bounded, canonical descriptor inventory.
    #[must_use]
    pub const fn entries(&self) -> &BTreeSet<RegistryInventoryEntry> {
        &self.entries
    }
}

/// A registry object that `PostgreSQL` protects from future retention actions.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct RegistryRetentionRoot {
    repository_path: RegistryNamespace,
    digest: Sha256Digest,
    kind: RegistryRetentionRootKind,
}

impl RegistryRetentionRoot {
    const fn new(
        repository_path: RegistryNamespace,
        digest: Sha256Digest,
        kind: RegistryRetentionRootKind,
    ) -> Self {
        Self {
            repository_path,
            digest,
            kind,
        }
    }

    fn inventory_entry(&self) -> RegistryInventoryEntry {
        RegistryInventoryEntry::new(self.repository_path.clone(), self.digest.clone())
    }
}

/// The durable reason an OCI descriptor remains protected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RegistryRetentionRootKind {
    /// An approved platform catalog image manifest or index.
    ApprovedCatalog,
    /// An approved project repository-image manifest or index.
    ApprovedRepositoryOciImage,
    /// An approved project release-agent manifest or index.
    ApprovedReleaseAgent,
    /// A pending, publishing, or verified publication intent.
    ActiveIntent,
    /// A platform-specific manifest required by an approved or verified index.
    PlatformManifest,
    /// Required OCI supply-chain evidence for an approved or verified intent.
    RequiredEvidence,
}

/// Counts from the durable notification inbox, with no notification payloads.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct RegistryNotificationBacklog {
    /// Notifications that have not been claimed.
    pub pending: u64,
    /// Notifications currently leased by a reconciler.
    pub claimed: u64,
    /// Claimed notifications whose lease has expired and may be retried.
    pub expired_claims: u64,
    /// Terminally processed notifications.
    pub processed: u64,
    /// Terminally rejected notifications.
    pub rejected: u64,
}

/// Durable registry lifecycle and inbox observability counters.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct RegistryOperationalMetrics {
    /// Pending publication intents.
    pub pending_publications: u64,
    /// Publication intents currently being published.
    pub publishing_publications: u64,
    /// Verified publication intents awaiting approval.
    pub verified_publications: u64,
    /// Approved publication intents.
    pub approved_publications: u64,
    /// Retired historical publication intents.
    pub retired_publications: u64,
    /// Publications marked unavailable by reconciliation.
    pub missing_publications: u64,
    /// Bounded durable inbox state counts.
    pub notification_backlog: RegistryNotificationBacklog,
}

/// PostgreSQL-derived input to the provider-neutral retention evaluator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryRetentionSnapshot {
    intents: Vec<PublicationIntent>,
    metrics: RegistryOperationalMetrics,
}

impl RegistryRetentionSnapshot {
    /// Creates one read-only durable snapshot.
    #[must_use]
    pub const fn new(intents: Vec<PublicationIntent>, metrics: RegistryOperationalMetrics) -> Self {
        Self { intents, metrics }
    }
}

/// Stable, non-destructive comparison of durable roots and registry inventory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RegistryRetentionReport {
    schema_version: u16,
    mode: RegistryRetentionReportMode,
    inventory_entries: usize,
    retention_roots: Vec<RegistryRetentionRoot>,
    missing_from_inventory: Vec<RegistryRetentionRoot>,
    unreferenced_inventory: Vec<RegistryInventoryEntry>,
    observability: RegistryOperationalMetrics,
    schema_scope: RegistryRetentionSchemaScope,
}

/// This report is deliberately observational; it cannot delete Zot content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RegistryRetentionReportMode {
    /// Lists protected and drifting descriptors without a destructive action.
    ReportOnly,
}

/// Documents the durable OCI ownership represented by the current schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
// These independent capability flags make a partially migrated retention
// schema explicit in operator JSON; combining them into states would hide gaps.
#[allow(clippy::struct_excessive_bools)]
pub struct RegistryRetentionSchemaScope {
    /// Platform catalog image publications are represented.
    pub platform_catalog: bool,
    /// Project repository-image publications are represented.
    pub repository_oci_images: bool,
    /// Project release-agent publications are represented.
    pub release_agents: bool,
    /// Separate generic build-image roots are not represented yet.
    pub generic_build_images: bool,
    /// Separate generic release-artifact OCI roots are not represented yet.
    pub generic_release_artifacts: bool,
}

impl RegistryRetentionReport {
    /// Evaluates a bounded external inventory against durable `PostgreSQL` roots.
    ///
    /// The function has no registry client and no mutation capability. It is
    /// therefore suitable for Zot, another OCI provider, or a saved inventory
    /// document supplied to the operator command.
    #[must_use]
    pub fn evaluate(snapshot: RegistryRetentionSnapshot, inventory: RegistryInventory) -> Self {
        let RegistryRetentionSnapshot { intents, metrics } = snapshot;
        let roots = retention_roots(&intents);
        let observed = inventory.entries;
        let missing_from_inventory = roots
            .iter()
            .filter(|root| !observed.contains(&root.inventory_entry()))
            .cloned()
            .collect();
        let protected = roots
            .iter()
            .map(RegistryRetentionRoot::inventory_entry)
            .collect::<BTreeSet<_>>();
        let unreferenced_inventory = observed.difference(&protected).cloned().collect();
        Self {
            schema_version: 1,
            mode: RegistryRetentionReportMode::ReportOnly,
            inventory_entries: observed.len(),
            retention_roots: roots.into_iter().collect(),
            missing_from_inventory,
            unreferenced_inventory,
            observability: metrics,
            schema_scope: RegistryRetentionSchemaScope {
                platform_catalog: true,
                repository_oci_images: true,
                release_agents: true,
                generic_build_images: false,
                generic_release_artifacts: false,
            },
        }
    }
}

fn retention_roots(intents: &[PublicationIntent]) -> BTreeSet<RegistryRetentionRoot> {
    let mut roots = BTreeSet::new();
    for intent in intents {
        let namespace = intent.reference().namespace().clone();
        let primary_kind = match intent.state() {
            PublicationState::Approved => Some(match intent.claim().owner() {
                RegistryOwner::PlatformImage { .. } => RegistryRetentionRootKind::ApprovedCatalog,
                RegistryOwner::RepositoryOciImage { .. } => {
                    RegistryRetentionRootKind::ApprovedRepositoryOciImage
                }
                RegistryOwner::ReleaseAgent { .. } => {
                    RegistryRetentionRootKind::ApprovedReleaseAgent
                }
            }),
            PublicationState::Pending
            | PublicationState::Publishing
            | PublicationState::Verified => Some(RegistryRetentionRootKind::ActiveIntent),
            PublicationState::Retired | PublicationState::Missing => None,
        };
        let Some(primary_kind) = primary_kind else {
            continue;
        };
        roots.insert(RegistryRetentionRoot::new(
            namespace.clone(),
            intent.reference().digest().clone(),
            primary_kind,
        ));
        if let Some(verification) = intent.verification() {
            for platform in verification.platforms() {
                roots.insert(RegistryRetentionRoot::new(
                    namespace.clone(),
                    platform.descriptor().digest().clone(),
                    RegistryRetentionRootKind::PlatformManifest,
                ));
            }
            for referrer in verification.evidence().referrers() {
                roots.insert(RegistryRetentionRoot::new(
                    namespace.clone(),
                    referrer.descriptor().digest().clone(),
                    RegistryRetentionRootKind::RequiredEvidence,
                ));
            }
        }
    }
    roots
}

/// Retention report input validation failure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RegistryRetentionReportError {
    /// The inventory document has an unsupported schema revision.
    #[error("unsupported registry inventory schema version {0}")]
    UnsupportedInventorySchema(u16),
    /// The inventory exceeds its bounded descriptor count.
    #[error("registry inventory exceeds the bounded descriptor count")]
    InventoryTooLarge,
}
