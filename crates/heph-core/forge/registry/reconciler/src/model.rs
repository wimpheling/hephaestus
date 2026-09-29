//! Public reconciliation data types.

use registry_domain::{
    OciDescriptor, PlatformDescriptor, PublicationIntentId, PublicationState, RegistryNamespace,
    SupplyChainEvidence, VerifiedPublication,
};

/// Exact target retained from a bounded Zot observation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedTarget {
    /// Observed immutable digest.
    pub digest: registry_domain::Sha256Digest,
    /// Observed OCI media type.
    pub media_type: registry_domain::OciMediaType,
}

/// Adapter-neutral claimed notification consumed by reconciliation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimedNotification {
    /// Durable inbox identity.
    pub id: uuid::Uuid,
    /// Opaque lease ownership token returned unchanged on completion.
    pub lease_token: uuid::Uuid,
    /// Bounded raw path retained when canonical namespace parsing failed.
    pub repository_path: String,
    /// Canonical namespace when the observation addressed one.
    pub namespace: Option<RegistryNamespace>,
    /// Optional exact manifest target from the observation.
    pub target: Option<ObservedTarget>,
}

/// Terminal inbox result committed only after actions succeed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotificationCompletion {
    /// The observation was authoritatively reconciled.
    Processed,
    /// The observation was bounded but did not address an owned namespace.
    Rejected {
        /// Stable non-sensitive reason code.
        failure_code: String,
    },
}

/// Opaque failure returned by a reconciliation port.
///
/// The public contract deliberately avoids carrying registry credentials,
/// callback payloads, SQL text, or remote response bodies across layers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("registry reconciliation dependency is unavailable")]
pub struct ReconciliationPortError;

/// Exact Zot graph read for one immutable digest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ZotInspection {
    /// Zot no longer serves the requested immutable manifest digest.
    Missing,
    /// Zot served a response for the digest, but its exact descriptors or
    /// referrer graph were malformed, inconsistent, or outside safety bounds.
    Invalid,
    /// Zot returned the manifest, platform descriptors, and referrer subject
    /// graph read back for the requested digest.
    Present {
        /// Top-level manifest or index descriptor.
        manifest: OciDescriptor,
        /// Platform-specific manifests referenced by the top-level content.
        platforms: Vec<PlatformDescriptor>,
        /// Referrers and their declared subject, read from Zot.
        evidence: SupplyChainEvidence,
    },
}
/// Stable semantic reason an immutable Zot graph cannot be trusted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Inconsistency {
    /// The exact approved digest is no longer present in Zot.
    ContentMissing,
    /// The graph could not form a valid immutable verification record.
    InvalidZotGraph,
    /// Zot's manifest descriptor differs from the durable expected descriptor.
    ManifestDescriptorMismatch,
    /// Zot's evidence conflicts with already retained immutable evidence.
    ImmutableEvidenceMismatch,
    /// Zot's graph does not satisfy the publication's supply-chain policy.
    SupplyChainPolicyViolation,
}

/// A requested lifecycle mutation for a separately authorized executor.
///
/// There is intentionally no approval variant. A notification or scheduled
/// pass may establish verified evidence, mark an approved digest missing, or
/// propose recovery; policy approval remains an explicit later operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReconciliationAction {
    /// Persist newly read immutable evidence as `verified`, never `approved`.
    RecordVerified {
        /// Publication that can record the evidence.
        intent_id: PublicationIntentId,
        /// Exact evidence read back from Zot.
        verification: VerifiedPublication,
    },
    /// Fail closed because a previously approved digest is absent or invalid.
    MarkMissing {
        /// Formerly approved publication.
        intent_id: PublicationIntentId,
        /// Why the exact remote graph is unusable.
        reason: Inconsistency,
    },
    /// Restore a missing approval only when its original evidence is exact.
    RestoreVerified {
        /// Missing publication that can be restored by an executor.
        intent_id: PublicationIntentId,
        /// Exact immutable evidence that permits recovery.
        verification: VerifiedPublication,
    },
    /// Record an observation whose digest/media type does not identify an
    /// existing intent. It is diagnostic only and never changes lifecycle.
    ObservedDifferentTarget {
        /// Namespace containing the observation.
        namespace: RegistryNamespace,
    },
    /// Record a bounded unknown or unclaimed namespace observation.
    OrphanNamespace {
        /// Bounded repository path retained by the notification inbox.
        repository_path: String,
    },
    /// Surface a non-approved inconsistency for investigation without making
    /// an illegal lifecycle transition.
    Investigate {
        /// Publication with inconsistent Zot content.
        intent_id: PublicationIntentId,
        /// Why reconciliation cannot safely verify it.
        reason: Inconsistency,
    },
}

/// Per-intent result of an exact Zot inspection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntentReconciliation {
    /// Durable intent inspected.
    pub intent_id: PublicationIntentId,
    /// State read before any proposed action is executed.
    pub state: PublicationState,
    /// Typed follow-up actions. They have not been applied.
    pub actions: Vec<ReconciliationAction>,
}

/// Reduction result for one claimed Zot notification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationReduction {
    /// Claim reduced by this result.
    pub notification_id: uuid::Uuid,
    /// Terminal inbox completion safe to commit after reduction.
    pub completion: NotificationCompletion,
    /// Per-intent authoritative outcomes.
    pub intents: Vec<IntentReconciliation>,
    /// Diagnostic or follow-up actions that have not been applied.
    pub actions: Vec<ReconciliationAction>,
}

/// Result of a scheduled full authoritative reconciliation pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthoritativeReconciliation {
    /// Every inspected durable intent.
    pub intents: Vec<IntentReconciliation>,
}
