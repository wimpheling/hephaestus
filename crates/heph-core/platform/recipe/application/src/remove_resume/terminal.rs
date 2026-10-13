mod inventory;
use super::{RemoveContinuation, RemoveResumeAdmission};
use crate::{
    DeploymentAdmission, DeploymentError, OriginalInstanceClosure, PendingInstanceDrainClosure,
    PreparedInstanceDeleteHistory, PreparedInstanceDeleteHistoryState,
    PreparedInstanceDetachRecovery, RemoveResumeConfiguration, ResourceAction,
};

impl RemoveResumeAdmission {
    /// Compares native143's complete current active and terminal/Detach inventories.
    ///
    /// Every supplied active claim must match current snapshot activity exactly;
    /// every entry must match full checked intent/original/config/current command.
    /// The adapter must qualify actual current rights, historical protected prefixes,
    /// actual Applied history and immutable FIRST context independently. An older
    /// saved endpoint cannot be substituted for this current observation.
    ///
    /// Empty vectors mean absent optional inventories; at least one must be present.
    /// Wire decoders must separately reject present-empty, null or unknown forms.
    /// Old constructors remain strict and never infer a Delete continuation.
    ///
    /// # Errors
    /// Rejects incomplete, duplicate, foreign, cross-inventory or substituted data,
    /// unsupported actions, generation drift and inactive Delete without exact
    /// committed/Applied history. Pure construction grants no admission or IO.
    pub fn with_instance_terminal_history(
        admission: DeploymentAdmission,
        continuations: Vec<RemoveContinuation>,
        closures: Vec<OriginalInstanceClosure>,
        pending: Vec<PendingInstanceDrainClosure>,
        detach: Vec<PreparedInstanceDetachRecovery>,
        terminal: Vec<PreparedInstanceDeleteHistory>,
        configuration: &RemoveResumeConfiguration,
    ) -> Result<Self, DeploymentError> {
        inventory::current(&admission, &continuations)?;
        inventory::entries(
            &admission,
            &continuations,
            &closures,
            &pending,
            &detach,
            &terminal,
            configuration,
        )?;
        // Only positively matched Delete entries are taken out of the old142
        // comparison. Their complete active/inactive accounting was checked above.
        let old = continuations
            .iter()
            .filter(|item| item.claim.action != ResourceAction::Delete)
            .cloned()
            .collect();
        let mut result = Self::with_pending_instance_drains(admission, old, closures, pending)?;
        result.continuations = continuations;
        result.detach = detach;
        result.terminal = terminal;
        Ok(result)
    }
    /// Returns exact original instance Detach Recovery comparison projections.
    #[must_use]
    pub fn instance_detach_recovery(&self) -> &[PreparedInstanceDetachRecovery] {
        &self.detach
    }
    /// Returns complete Delete history, including positively observed inactive originals.
    ///
    /// Applied entries are comparison data from a qualified adapter, not receipts.
    #[must_use]
    pub fn instance_delete_history(&self) -> &[PreparedInstanceDeleteHistory] {
        &self.terminal
    }
}

fn active_match(
    continuations: &[RemoveContinuation],
    claim: &crate::EffectClaim,
    closure: &OriginalInstanceClosure,
    state: PreparedInstanceDeleteHistoryState,
) -> Result<(), DeploymentError> {
    let actual = continuations
        .iter()
        .find(|item| item.claim.resource == claim.resource);
    let valid = match state {
        PreparedInstanceDeleteHistoryState::Active => actual
            .is_some_and(|item| item.claim == *claim && item.closure.as_ref() == Some(closure)),
        PreparedInstanceDeleteHistoryState::Applied => actual.is_none(),
    };
    if !valid {
        return Err(DeploymentError::ReconciliationRequired);
    }
    Ok(())
}
