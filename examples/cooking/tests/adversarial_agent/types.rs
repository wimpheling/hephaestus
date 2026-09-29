// Reuse the adversarial facade imports across the focused phases.
#[allow(unused_imports)]
use super::*;
/// Result of one denied agent operation, retaining only opaque IDs and counts.
#[derive(Debug, Clone, Copy)]
pub(crate) struct AdversarialAgentProbe {
    /// The distinct adversarial mailbox event.
    pub event_id: Uuid,
    /// The run created for the event.
    pub run_id: Uuid,
    /// The fresh rule whose destination is deliberately incompatible.
    pub mismatched_rule_id: Uuid,
    /// Number of durable deny decisions for the run/rule.
    pub deny_decisions: i64,
    /// Number of durable substitution uses; must remain zero.
    pub substitution_uses: i64,
}

pub(crate) type RunAuditSummary = (Uuid, Uuid, String, i64, i64, i64, i64, i64);

/// The immutable rule and binding identities selected for an imported
/// instance. Callers use these IDs to register a broker adapter before the
/// daemon is restarted.
#[derive(Debug, Clone, Copy)]
pub(crate) struct BrokeredRuleSpec {
    /// Immutable broker rule identity claimed by the guest.
    pub rule_id: Uuid,
    /// Binding revision selected by the authenticated bind operation.
    pub binding_id: Uuid,
    /// Immutable instance revision owning the binding and rule.
    pub instance_revision_id: Uuid,
    /// Secret version pinned by that binding.
    pub secret_version_id: Uuid,
    /// Slot receiving the binding.
    pub slot: &'static str,
    /// Exact HTTPS origin authorized for the rule.
    pub destination: &'static str,
}

/// A prepared cooking instance together with both broker rule specs.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PreparedBrokeredInstance {
    /// Imported instance and final immutable revision.
    pub instance: PreparedCookingInstance,
    /// Model binding/rule selected before import.
    pub model: BrokeredRuleSpec,
    /// Relay binding/rule selected before import.
    pub relay: BrokeredRuleSpec,
}

/// Stable rule identities registered by the shared upstream dispatcher.
#[derive(Debug, Clone, Copy)]
pub(crate) struct BrokeredRuleIds {
    /// Rule identity for the model destination.
    pub model: Uuid,
    /// Rule identity for the relay destination.
    pub relay: Uuid,
}

pub(crate) struct CanonicalBrokeredSecrets {
    pub(crate) model_import: Uuid,
    pub(crate) relay_import: Uuid,
    pub(crate) model_version: Uuid,
    pub(crate) relay_version: Uuid,
}
