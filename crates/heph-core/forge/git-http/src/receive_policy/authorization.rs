use crate::Principal;
use git_capability_domain::{ReceiveUpdate, RepositoryId};

use super::{GuardedReceiveError, ReceivePolicyError, TrustedPathChange, TrustedReceiveProposal};

/// Injectable trusted policy boundary for an inspected receive proposal.
pub trait ReceivePolicyGuard: Send + Sync {
    /// Atomically authorizes every ref transition and changed path.
    ///
    /// An implementation must return success only if the whole batch may be
    /// made canonical.
    ///
    /// # Errors
    ///
    /// Returns a fail-closed denial without mutating repository state.
    fn authorize(&self, proposal: &TrustedReceiveProposal) -> Result<(), ReceivePolicyError>;
}

/// Git-capability grammar enforcement for trusted receive facts.
#[derive(Debug, Clone, Copy, Default)]
pub struct CapabilityReceivePolicyGuard;

impl ReceivePolicyGuard for CapabilityReceivePolicyGuard {
    fn authorize(&self, proposal: &TrustedReceiveProposal) -> Result<(), ReceivePolicyError> {
        let scope = proposal.context().scope();
        let limits = scope.transfer_limits();
        if proposal.request_bytes > limits.request_bytes()
            || proposal.pack_bytes > limits.pack_bytes()
            || proposal.object_count > limits.object_count()
        {
            return Err(ReceivePolicyError::TransferLimitExceeded);
        }
        if proposal.updates.is_empty() || proposal.updates.len() > usize::from(limits.ref_updates())
        {
            return Err(ReceivePolicyError::InvalidRefUpdateCount);
        }
        if let Some(expected_parent) = proposal.context().expected_parent() {
            if proposal.updates.len() != 1
                || proposal.updates[0].old_object.as_deref() != Some(expected_parent)
            {
                return Err(ReceivePolicyError::ExpectedParentMismatch);
            }
        }

        for (index, update) in proposal.updates().iter().enumerate() {
            let changed_paths = update
                .changed_paths
                .iter()
                .map(TrustedPathChange::as_borrowed)
                .collect::<Vec<_>>();
            if !scope.allows_receive(&ReceiveUpdate {
                reference: &update.reference,
                transition: update.transition,
                changed_paths: &changed_paths,
            }) {
                return Err(ReceivePolicyError::ScopeDenied {
                    update_index: index,
                });
            }
        }
        Ok(())
    }
}

/// Proof that the exact proposal passed its configured trusted guard.
///
/// Construction is private so callers can obtain this value only through
/// [`authorize_before_canonical_mutation`].
#[derive(Debug)]
pub struct ReceiveMutationPermit {
    repository_id: RepositoryId,
}

impl ReceiveMutationPermit {
    /// Returns the exact repository authorized for canonical mutation.
    #[must_use]
    pub const fn repository_id(&self) -> RepositoryId {
        self.repository_id
    }
}

/// Authorizes an exact-runtime receive, then invokes canonical mutation.
///
/// Existing human pushes bypass the runtime capability guard. Runtime pushes
/// fail closed when no guard or trusted proposal is installed. A policy denial
/// always returns before `mutate` is invoked.
///
/// # Errors
///
/// Returns a policy error before mutation or wraps an error returned by the
/// canonical mutation callback.
pub fn authorize_before_canonical_mutation<T, E>(
    principal: &Principal,
    guard: Option<&dyn ReceivePolicyGuard>,
    proposal: Option<&TrustedReceiveProposal>,
    mutate: impl FnOnce(Option<ReceiveMutationPermit>) -> Result<T, E>,
) -> Result<T, GuardedReceiveError<E>> {
    let Principal::Runtime(runtime) = principal else {
        return mutate(None).map_err(GuardedReceiveError::Mutation);
    };
    let guard = guard.ok_or(ReceivePolicyError::RuntimeGuardUnavailable)?;
    let proposal = proposal.ok_or(ReceivePolicyError::TrustedProposalUnavailable)?;
    if runtime.runtime_session_id() != proposal.context.runtime_session_id()
        || runtime.authorization_snapshot_id() != proposal.context.authorization_snapshot_id()
    {
        return Err(ReceivePolicyError::RuntimeBindingMismatch.into());
    }
    guard.authorize(proposal)?;
    mutate(Some(ReceiveMutationPermit {
        repository_id: proposal.context().repository_id(),
    }))
    .map_err(GuardedReceiveError::Mutation)
}
