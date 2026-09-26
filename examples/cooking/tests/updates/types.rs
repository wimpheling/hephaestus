// Reuse the update facade imports across focused lifecycle phases.
#[allow(unused_imports)]
use super::*;
/// Authentication and durable resources used by the update RPC helpers.
pub(crate) struct CookingUpdateContext<'a> {
    /// Application database used for lifecycle and deferred-work assertions.
    pub pool: &'a PgPool,
    /// Running daemon exposing the instance Connect service.
    pub running: &'a hephaestus_app::RunningHephaestus,
    /// Instance whose active revision is being advanced.
    pub instance_id: Uuid,
    /// Gateway whose exact mailbox publication identifies cooking events.
    pub gateway: &'a super::super::GatewayGoldenFixture,
    /// Revision active before this update sequence.
    pub current_revision_id: Uuid,
    /// Owner used by the mediator authorization policy.
    pub owner: Uuid,
    /// Creates a fresh assertion for each exact RPC audience.
    pub rpc_token: &'a (dyn Fn(&str) -> String + Send + Sync),
    /// Exact source rule identities active on this revision.
    pub brokered_rule_ids: BrokeredRuleIds,
    /// Bound for each lifecycle wait.
    pub timeout: Duration,
}

/// Stable IDs returned by the production `CreateUpdate` command.
// The type intentionally groups the durable IDs returned by one RPC;
// each field names a different persisted identity.
#[allow(clippy::struct_field_names)]
#[derive(Debug, Clone, Copy)]
pub(crate) struct UpdateIds {
    /// Durable update identity.
    pub update_id: Uuid,
    /// Candidate instance revision created by the command.
    pub candidate_revision_id: Uuid,
    /// Fresh model rule identity carried by the candidate revision.
    pub model_rule_id: Uuid,
    /// Fresh relay rule identity carried by the candidate revision.
    pub relay_rule_id: Uuid,
}

/// The two brokered rules carried by one immutable cooking revision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BrokeredRuleIds {
    pub model: Uuid,
    pub relay: Uuid,
}

/// Published agents and explicit rule mappings used by one update sequence.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CookingUpdateCandidates {
    pub migrate_release_agent_id: Uuid,
    pub rollback_release_agent_id: Uuid,
    pub abnormal_release_agent_id: Uuid,
    pub migrate_rule_ids: BrokeredRuleIds,
    pub rollback_rule_ids: BrokeredRuleIds,
    pub abnormal_rule_ids: BrokeredRuleIds,
}

impl BrokeredRuleIds {
    pub fn fresh() -> Self {
        Self {
            model: Uuid::new_v4(),
            relay: Uuid::new_v4(),
        }
    }

    pub(crate) fn copies_from(self, source: Self) -> Vec<BrokeredRuleCopy> {
        vec![
            BrokeredRuleCopy {
                source_rule_id: opaque(source.model).into(),
                candidate_rule_id: opaque(self.model).into(),
                ..Default::default()
            },
            BrokeredRuleCopy {
                source_rule_id: opaque(source.relay).into(),
                candidate_rule_id: opaque(self.relay).into(),
                ..Default::default()
            },
        ]
    }
}

/// Durable update lifecycle projection used in assertions and diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UpdateState {
    /// `agent_updates.state`.
    pub update: String,
    /// `agent_instances.state`.
    pub instance: String,
    /// Current durable run gate value.
    pub run_gate_open: bool,
    /// Active instance revision, if one is recorded.
    pub active_revision_id: Option<Uuid>,
    /// Candidate revision attached to this update.
    pub candidate_revision_id: Uuid,
}

pub(crate) type DeferredEventProjection = (
    String,
    Option<Uuid>,
    Option<Uuid>,
    Option<String>,
    Option<String>,
);

pub(crate) fn deferred_event_is_terminal_success(
    projection: &DeferredEventProjection,
    candidate_revision_id: Uuid,
) -> bool {
    let (disposition, Some(_run_id), revision_id, run_state, outcome) = projection else {
        return false;
    };
    *revision_id == Some(candidate_revision_id)
        && disposition == "delivered"
        && run_state.as_deref() == Some("cleaned_up")
        && outcome.as_deref() == Some("succeeded")
}

/// IDs and active revision returned by the complete three-variant update
/// sequence.
#[derive(Debug, Clone, Copy)]
pub(crate) struct UpdateSequence {
    /// Migration update that activated the v2 candidate.
    pub migration: UpdateIds,
    /// Abnormal hook update recovered by operator rejection.
    pub abnormal: UpdateIds,
    /// The completed pre-rotation relay event whose old lease remains in history.
    pub relay_run_id: Uuid,
    /// Relay version rotation performed before candidate rule cloning.
    pub relay_rotation: super::super::cooking::CredentialRotation,
}

/// Secret versions observed across one held v1 request and its later dispatch.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CredentialRotation {
    /// Version pinned by the in-flight v1 lease.
    pub pinned_version_id: Uuid,
    /// Version selected by the later dispatch after rotation commits.
    pub rotated_version_id: Uuid,
}
