use runtime_types::{AgentInstanceId, RunId, VolumeId};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Closed vocabulary of durable work, including terminal historical entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstanceSafetyWorkKind {
    /// Persistent mailbox consumer.
    Mailbox,
    /// Accepted or terminal delivery, independent of Run cleanup.
    Delivery,
    /// Delivery attempt, including a leased attempt requiring terminal proof.
    DeliveryAttempt,
    /// Original invocation/Git request, including dispatched history.
    RunRequest,
    /// Durable deferred trigger.
    DeferredTrigger,
    /// Update and its potentially multiple hook attempts.
    Update,
}
/// Safe work identity; statuses and payloads remain inside the owning adapter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstanceSafetyWork {
    /// Exact durable work category.
    pub kind: InstanceSafetyWorkKind,
    /// Immutable backend row identity, not caller authority.
    pub id: Uuid,
}
/// Complete global reference inventory; values confer no IO authority.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstanceSafetyInventory {
    /// Every Run for this consumer, regardless of state, kind or host.
    pub runs: Vec<RunId>,
    /// Every lease for the consumer or any of its Runs, including released rows.
    pub leases: Vec<Uuid>,
    /// Every mount grant across all revisions, including revoked history.
    pub mount_grants: Vec<Uuid>,
    /// Every volume referenced by any revision, scalar pointer or lease history.
    pub volumes: Vec<VolumeId>,
    /// All consumers of those volumes; other live consumers can hold volume Retain.
    pub consumers: Vec<AgentInstanceId>,
    /// Complete mailbox, delivery, request, trigger and update reference history.
    pub work: Vec<InstanceSafetyWork>,
}
