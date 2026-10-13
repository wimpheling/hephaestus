//! Conservative positive pairs from separate actual immutable reader snapshots.

use super::{ManagedRunCommandExecutor, OrchestratorError, held};
use heph_run::StartRun;
use run_postgres::{
    QualifiedInvocationRoutingDecision as Invocation,
    QualifiedInvocationRoutingHoldReason as InvocationHold,
    QualifiedOrdinaryRoutingDecision as Ordinary,
    QualifiedOrdinaryRoutingHoldReason as OrdinaryHold,
};
use runtime_types::RunId;

pub(super) enum Selection {
    Invocation,
    LegacyOriginal,
    LegacyProspective,
}

impl ManagedRunCommandExecutor {
    pub(super) async fn select_start(
        &self,
        command: &StartRun,
    ) -> Result<Selection, OrchestratorError> {
        let invocation = self.reader.inspect_start(command).await?;
        let ordinary = self.reader.inspect_ordinary_start(command).await?;
        select(&invocation, &ordinary)
    }

    pub(super) async fn select_original(&self, run: RunId) -> Result<Selection, OrchestratorError> {
        let invocation = self.reader.inspect_original(run).await?;
        let ordinary = self.reader.inspect_ordinary_original(run).await?;
        select(&invocation, &ordinary)
    }
}

fn select(invocation: &Invocation, ordinary: &Ordinary) -> Result<Selection, OrchestratorError> {
    // Both reads precede selection. A broad other-family Held never qualifies
    // anything: the selected branch always has its own complete positive graph.
    // An unborn result remains only a prospect; actual birth rechecks all pins.
    // Separate snapshots can race and cause Held/retry. They are not an atomic
    // observation or a lock; persisted profile checks must precede controller IO.
    match (invocation, ordinary) {
        (Invocation::Qualified(_), Ordinary::Held(OrdinaryHold::UnsupportedProducer)) => {
            Ok(Selection::Invocation)
        }
        (Invocation::Held(InvocationHold::UnsupportedProfile), Ordinary::Qualified(_)) => {
            Ok(Selection::LegacyOriginal)
        }
        (Invocation::Held(InvocationHold::MissingRun), Ordinary::ProspectivelyUnborn(_)) => {
            Ok(Selection::LegacyProspective)
        }
        (_, Ordinary::Held(OrdinaryHold::CanonicalOriginalFacadeRequired)) => Err(held(
            "managed Run routing is held: ordinary canonical original facade is unsupported",
        )),
        (Invocation::Held(InvocationHold::ForeignOwner), _)
        | (_, Ordinary::Held(OrdinaryHold::ForeignOwner)) => Err(held(
            "managed Run routing is held: original belongs to a foreign provider owner",
        )),
        (Invocation::Held(InvocationHold::WorkerRoleRequired), _)
        | (_, Ordinary::Held(OrdinaryHold::WorkerRoleRequired)) => Err(held(
            "managed Run routing is held: actual dedicated worker role is required",
        )),
        _ => Err(held(
            "managed Run routing is held: unsupported, contradictory or racing original evidence",
        )),
    }
}

#[cfg(test)]
mod tests;
