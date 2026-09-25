//! Trusted receive-policy enforcement before canonical ref mutation.
//!
//! This module deliberately accepts already-inspected receive facts. Producing
//! those facts safely requires a quarantined pack and trusted repository
//! inspection; parsing client claims is not sufficient. The caller must invoke
//! [`authorize_before_canonical_mutation`] before making proposed objects or
//! refs canonical.

use crate::Principal;
use git_capability_domain::{
    ChangedPathGlob, GitCapabilityScope, GitCapabilityScopeInput, GitOperation, PathChange,
    ReceiveUpdate, RefGlob, RefTransition, RefUpdatePolicy, RepositoryId, TransferLimits,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Immutable authority and identity resolved for one runtime receive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedRuntimeReceiveContext {
    scope: Arc<GitCapabilityScope>,
    runtime_session_id: Arc<str>,
    authorization_snapshot_id: Arc<str>,
    evaluated_at_unix_seconds: i64,
    expected_parent: Option<Arc<str>>,
}

impl ResolvedRuntimeReceiveContext {
    /// Creates a receive context from a resolved immutable capability scope.
    ///
    /// # Errors
    ///
    /// Returns an error when the identity fields are empty, the scope is not
    /// receive-capable, or it was already expired when resolved.
    pub fn new(
        scope: Arc<GitCapabilityScope>,
        runtime_session_id: impl Into<Arc<str>>,
        authorization_snapshot_id: impl Into<Arc<str>>,
        evaluated_at_unix_seconds: i64,
    ) -> Result<Self, ReceivePolicyError> {
        let runtime_session_id = runtime_session_id.into();
        let authorization_snapshot_id = authorization_snapshot_id.into();
        if runtime_session_id.is_empty() || authorization_snapshot_id.is_empty() {
            return Err(ReceivePolicyError::InvalidResolvedContext);
        }
        if !scope.operations().contains(&GitOperation::Receive)
            || !scope.is_active_at(evaluated_at_unix_seconds)
        {
            return Err(ReceivePolicyError::InvalidResolvedContext);
        }
        Ok(Self {
            scope,
            runtime_session_id,
            authorization_snapshot_id,
            evaluated_at_unix_seconds,
            expected_parent: None,
        })
    }

    /// Creates a receive context and binds an optional exact old commit.
    ///
    /// # Errors
    ///
    /// Returns an error for an empty or non-hex expected parent, in addition
    /// to the checks performed by [`Self::new`].
    pub fn new_with_expected_parent(
        scope: Arc<GitCapabilityScope>,
        runtime_session_id: impl Into<Arc<str>>,
        authorization_snapshot_id: impl Into<Arc<str>>,
        evaluated_at_unix_seconds: i64,
        expected_parent: Option<impl Into<Arc<str>>>,
    ) -> Result<Self, ReceivePolicyError> {
        let mut context = Self::new(
            scope,
            runtime_session_id,
            authorization_snapshot_id,
            evaluated_at_unix_seconds,
        )?;
        context.expected_parent = expected_parent.map(Into::into);
        if context.expected_parent.as_deref().is_some_and(|parent| {
            !matches!(parent.len(), 40 | 64)
                || !parent
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        }) {
            return Err(ReceivePolicyError::InvalidResolvedContext);
        }
        Ok(context)
    }

    /// Returns the repository fixed by the resolved scope.
    #[must_use]
    pub fn repository_id(&self) -> RepositoryId {
        self.scope.repository_id()
    }

    /// Returns the complete immutable scope resolved by the trusted host.
    #[must_use]
    pub fn scope(&self) -> &GitCapabilityScope {
        &self.scope
    }

    /// Returns the exact runtime-session binding.
    #[must_use]
    pub fn runtime_session_id(&self) -> &str {
        &self.runtime_session_id
    }

    /// Returns the immutable authorization-snapshot binding.
    #[must_use]
    pub fn authorization_snapshot_id(&self) -> &str {
        &self.authorization_snapshot_id
    }

    /// Returns when trusted authorization state was evaluated.
    #[must_use]
    pub const fn evaluated_at_unix_seconds(&self) -> i64 {
        self.evaluated_at_unix_seconds
    }

    /// Returns the exact old commit required by this receive, when any.
    #[must_use]
    pub fn expected_parent(&self) -> Option<&str> {
        self.expected_parent.as_deref()
    }

    /// Returns whether the immutable scope remains active at `unix_seconds`.
    #[must_use]
    pub fn is_active_at(&self, unix_seconds: i64) -> bool {
        self.scope.is_active_at(unix_seconds)
    }

    /// Returns the immutable transfer ceilings.
    #[must_use]
    pub fn transfer_limits(&self) -> TransferLimits {
        self.scope.transfer_limits()
    }

    /// Serializes this trusted context for the host-owned pre-receive hook.
    ///
    /// The representation contains authority but no credential plaintext. It
    /// must be supplied only through the cleared, host-constructed backend
    /// environment, never copied from an HTTP header or request body.
    ///
    /// # Errors
    ///
    /// Returns an error if the validated context cannot be serialized.
    pub fn to_hook_json(&self) -> Result<Vec<u8>, ReceivePolicyError> {
        serde_json::to_vec(&HookContext::from(self))
            .map_err(|_| ReceivePolicyError::InvalidHookContext)
    }

    /// Parses and revalidates a host-owned pre-receive hook context.
    ///
    /// # Errors
    ///
    /// Returns an error for malformed, non-normalized, expired, or otherwise
    /// invalid authority.
    pub fn from_hook_json(bytes: &[u8]) -> Result<Self, ReceivePolicyError> {
        let wire: HookContext =
            serde_json::from_slice(bytes).map_err(|_| ReceivePolicyError::InvalidHookContext)?;
        let context: Self = wire.try_into()?;
        if context.to_hook_json()?.as_slice() != bytes {
            return Err(ReceivePolicyError::InvalidHookContext);
        }
        Ok(context)
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct HookContext {
    scope: HookScope,
    runtime_session_id: String,
    authorization_snapshot_id: String,
    evaluated_at_unix_seconds: i64,
    expected_parent: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct HookScope {
    version: u16,
    repository_id: RepositoryId,
    operations: Vec<GitOperation>,
    ref_globs: Vec<String>,
    changed_path_globs: Vec<String>,
    update_policy: RefUpdatePolicy,
    expires_at_unix_seconds: i64,
    transfer_limits: HookTransferLimits,
}

#[derive(Debug, Serialize, Deserialize)]
struct HookTransferLimits {
    request_bytes: u64,
    pack_bytes: u64,
    object_count: u32,
    ref_updates: u16,
}

impl From<&ResolvedRuntimeReceiveContext> for HookContext {
    fn from(context: &ResolvedRuntimeReceiveContext) -> Self {
        let scope = &context.scope;
        let limits = scope.transfer_limits();
        Self {
            scope: HookScope {
                version: scope.version(),
                repository_id: scope.repository_id(),
                operations: scope.operations().to_vec(),
                ref_globs: scope
                    .ref_globs()
                    .iter()
                    .map(|glob| glob.as_str().to_owned())
                    .collect(),
                changed_path_globs: scope
                    .changed_path_globs()
                    .iter()
                    .map(|glob| glob.as_str().to_owned())
                    .collect(),
                update_policy: scope.update_policy(),
                expires_at_unix_seconds: scope.expires_at_unix_seconds(),
                transfer_limits: HookTransferLimits {
                    request_bytes: limits.request_bytes(),
                    pack_bytes: limits.pack_bytes(),
                    object_count: limits.object_count(),
                    ref_updates: limits.ref_updates(),
                },
            },
            runtime_session_id: context.runtime_session_id().to_owned(),
            authorization_snapshot_id: context.authorization_snapshot_id().to_owned(),
            evaluated_at_unix_seconds: context.evaluated_at_unix_seconds(),
            expected_parent: context.expected_parent().map(str::to_owned),
        }
    }
}

impl TryFrom<HookContext> for ResolvedRuntimeReceiveContext {
    type Error = ReceivePolicyError;

    fn try_from(wire: HookContext) -> Result<Self, Self::Error> {
        if wire.scope.version != git_capability_domain::GRAMMAR_VERSION {
            return Err(ReceivePolicyError::InvalidHookContext);
        }
        let transfer_limits = TransferLimits::new(
            wire.scope.transfer_limits.request_bytes,
            wire.scope.transfer_limits.pack_bytes,
            wire.scope.transfer_limits.object_count,
            wire.scope.transfer_limits.ref_updates,
        )
        .map_err(|_| ReceivePolicyError::InvalidHookContext)?;
        let ref_globs = wire
            .scope
            .ref_globs
            .into_iter()
            .map(RefGlob::parse_explicitly_broad)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| ReceivePolicyError::InvalidHookContext)?;
        let changed_path_globs = wire
            .scope
            .changed_path_globs
            .into_iter()
            .map(ChangedPathGlob::parse_explicitly_broad)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| ReceivePolicyError::InvalidHookContext)?;
        let scope = GitCapabilityScope::new(GitCapabilityScopeInput {
            repository_id: wire.scope.repository_id,
            operations: wire.scope.operations,
            ref_globs,
            changed_path_globs,
            update_policy: wire.scope.update_policy,
            expires_at_unix_seconds: wire.scope.expires_at_unix_seconds,
            transfer_limits,
        })
        .map_err(|_| ReceivePolicyError::InvalidHookContext)?;
        Self::new_with_expected_parent(
            Arc::new(scope),
            wire.runtime_session_id,
            wire.authorization_snapshot_id,
            wire.evaluated_at_unix_seconds,
            wire.expected_parent,
        )
    }
}

/// One owned changed-path fact derived from quarantined objects.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum TrustedPathChange {
    /// A new path.
    Addition(String),
    /// An existing path with changed content or mode.
    Modification(String),
    /// A removed path.
    Deletion(String),
    /// A rename or copy, including both authority-relevant paths.
    Rename {
        /// Original repository-relative path.
        from: String,
        /// New repository-relative path.
        to: String,
    },
}

impl TrustedPathChange {
    fn as_borrowed(&self) -> PathChange<'_> {
        match self {
            Self::Addition(path) => PathChange::Addition(path),
            Self::Modification(path) => PathChange::Modification(path),
            Self::Deletion(path) => PathChange::Deletion(path),
            Self::Rename { from, to } => PathChange::Rename { from, to },
        }
    }
}

/// One ref command whose transition and complete path delta were inspected
/// against trusted repository state.
#[derive(Debug, Clone)]
pub struct TrustedReceiveUpdate {
    reference: String,
    transition: RefTransition,
    changed_paths: Vec<TrustedPathChange>,
    old_object: Option<String>,
}

impl TrustedReceiveUpdate {
    /// Creates one owned, trusted receive update.
    #[must_use]
    pub fn new(
        reference: impl Into<String>,
        transition: RefTransition,
        changed_paths: Vec<TrustedPathChange>,
    ) -> Self {
        Self {
            reference: reference.into(),
            transition,
            changed_paths,
            old_object: None,
        }
    }

    /// Creates a trusted update including the canonical old object observed by
    /// the host-side quarantine inspector.
    #[must_use]
    pub fn new_with_old_object(
        reference: impl Into<String>,
        transition: RefTransition,
        changed_paths: Vec<TrustedPathChange>,
        old_object: Option<String>,
    ) -> Self {
        Self {
            reference: reference.into(),
            transition,
            changed_paths,
            old_object,
        }
    }
}

/// Complete trusted facts for one atomic receive transaction.
#[derive(Debug, Clone)]
pub struct TrustedReceiveProposal {
    context: ResolvedRuntimeReceiveContext,
    updates: Vec<TrustedReceiveUpdate>,
    request_bytes: u64,
    pack_bytes: u64,
    object_count: u32,
}

impl TrustedReceiveProposal {
    /// Creates a proposal from facts produced by trusted quarantine
    /// inspection.
    #[must_use]
    pub const fn new(
        context: ResolvedRuntimeReceiveContext,
        updates: Vec<TrustedReceiveUpdate>,
        request_bytes: u64,
        pack_bytes: u64,
        object_count: u32,
    ) -> Self {
        Self {
            context,
            updates,
            request_bytes,
            pack_bytes,
            object_count,
        }
    }

    /// Returns the immutable resolved runtime context.
    #[must_use]
    pub const fn context(&self) -> &ResolvedRuntimeReceiveContext {
        &self.context
    }

    /// Returns the complete proposed ref-command batch.
    #[must_use]
    pub fn updates(&self) -> &[TrustedReceiveUpdate] {
        &self.updates
    }
}

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
        let scope = &proposal.context.scope;
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
        if let Some(expected_parent) = proposal.context.expected_parent() {
            if proposal.updates.len() != 1
                || proposal.updates[0].old_object.as_deref() != Some(expected_parent)
            {
                return Err(ReceivePolicyError::ExpectedParentMismatch);
            }
        }

        for (index, update) in proposal.updates.iter().enumerate() {
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
        repository_id: proposal.context.repository_id(),
    }))
    .map_err(GuardedReceiveError::Mutation)
}

/// Receive capability denial before repository mutation.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ReceivePolicyError {
    /// Resolved identity/scope state was incomplete, expired, or not writable.
    #[error("resolved runtime receive context is invalid")]
    InvalidResolvedContext,
    /// Host-owned hook authority was malformed or failed revalidation.
    #[error("runtime receive hook context is invalid")]
    InvalidHookContext,
    /// Runtime receives remain disabled unless an enforcement guard is wired.
    #[error("runtime receive policy guard is unavailable")]
    RuntimeGuardUnavailable,
    /// No trusted quarantine inspection was supplied for the runtime receive.
    #[error("trusted runtime receive proposal is unavailable")]
    TrustedProposalUnavailable,
    /// The proposal belongs to a different runtime session or snapshot.
    #[error("runtime receive identity binding does not match")]
    RuntimeBindingMismatch,
    /// Trusted transfer facts exceed the immutable capability limits.
    #[error("runtime receive transfer limit was exceeded")]
    TransferLimitExceeded,
    /// The atomic command batch is empty or too large.
    #[error("runtime receive ref-update count is invalid")]
    InvalidRefUpdateCount,
    /// Trigger-safe publication did not update exactly the snapshotted old
    /// commit.
    #[error("runtime receive expected parent does not match")]
    ExpectedParentMismatch,
    /// At least one ref transition or changed path is outside the scope.
    #[error("runtime receive update {update_index} is outside capability scope")]
    ScopeDenied {
        /// Zero-based command position; ref and path values are not exposed.
        update_index: usize,
    },
}

/// Failure from guarded authorization or the canonical mutation callback.
#[derive(Debug, thiserror::Error)]
pub enum GuardedReceiveError<E> {
    /// Authorization failed before canonical mutation.
    #[error(transparent)]
    Policy(#[from] ReceivePolicyError),
    /// The already-authorized canonical mutation failed.
    #[error("canonical Git receive mutation failed")]
    Mutation(E),
}

#[cfg(test)]
mod tests {
    use super::{
        CapabilityReceivePolicyGuard, GuardedReceiveError, ReceivePolicyError,
        ResolvedRuntimeReceiveContext, TrustedPathChange, TrustedReceiveProposal,
        TrustedReceiveUpdate, authorize_before_canonical_mutation,
    };
    use crate::Principal;
    use git_capability_domain::{
        BranchRefPolicy, BranchUpdatePolicy, ChangedPathGlob, GitCapabilityScope,
        GitCapabilityScopeInput, GitOperation, RefGlob, RefMutationPermission, RefNamespacePolicy,
        RefTransition, RefUpdatePolicy, RepositoryId, TransferLimits,
    };
    use std::{cell::Cell, sync::Arc};
    use uuid::Uuid;

    fn scope() -> Arc<GitCapabilityScope> {
        Arc::new(
            GitCapabilityScope::new(GitCapabilityScopeInput {
                repository_id: RepositoryId::new(Uuid::nil()),
                operations: vec![GitOperation::Receive],
                ref_globs: vec![RefGlob::parse("refs/heads/runtime").expect("ref glob")],
                changed_path_globs: vec![ChangedPathGlob::parse("sessions/**").expect("path glob")],
                update_policy: RefUpdatePolicy {
                    branches: BranchRefPolicy {
                        updates: BranchUpdatePolicy::FastForwardOnly,
                        create: RefMutationPermission::Allow,
                        delete: RefMutationPermission::Deny,
                    },
                    tags: RefNamespacePolicy::default(),
                    other: RefNamespacePolicy::default(),
                },
                expires_at_unix_seconds: 2_000,
                transfer_limits: TransferLimits::new(1_024, 2_048, 32, 4).expect("transfer limits"),
            })
            .expect("receive scope"),
        )
    }

    fn proposal(changed_path: &str) -> TrustedReceiveProposal {
        let context = ResolvedRuntimeReceiveContext::new(scope(), "run-1", "snapshot-1", 1_000)
            .expect("resolved context");
        TrustedReceiveProposal::new(
            context,
            vec![TrustedReceiveUpdate::new(
                "refs/heads/runtime",
                RefTransition::Create,
                vec![TrustedPathChange::Addition(changed_path.to_owned())],
            )],
            512,
            1_024,
            2,
        )
    }

    #[test]
    fn scope_denial_happens_before_canonical_mutation() {
        let mutation_called = Cell::new(false);
        let result = authorize_before_canonical_mutation(
            &Principal::runtime("runtime", "run-1", "snapshot-1"),
            Some(&CapabilityReceivePolicyGuard),
            Some(&proposal("private/token.txt")),
            |_| {
                mutation_called.set(true);
                Ok::<_, ()>(())
            },
        );

        assert!(matches!(
            result,
            Err(GuardedReceiveError::Policy(
                ReceivePolicyError::ScopeDenied { update_index: 0 }
            ))
        ));
        assert!(!mutation_called.get());
    }

    #[test]
    fn full_batch_is_authorized_before_canonical_mutation() {
        let mutation_called = Cell::new(false);
        let proposal = proposal("sessions/run-1/message.json");
        let result = authorize_before_canonical_mutation(
            &Principal::runtime("runtime", "run-1", "snapshot-1"),
            Some(&CapabilityReceivePolicyGuard),
            Some(&proposal),
            |permit| {
                mutation_called.set(true);
                assert_eq!(
                    permit.expect("runtime permit").repository_id(),
                    proposal.context().repository_id()
                );
                Ok::<_, ()>("mutated")
            },
        );

        assert_eq!(result.expect("authorized mutation"), "mutated");
        assert!(mutation_called.get());
    }

    #[test]
    fn expected_parent_is_checked_before_canonical_mutation() {
        let expected = "1111111111111111111111111111111111111111";
        let context = ResolvedRuntimeReceiveContext::new_with_expected_parent(
            scope(),
            "run-1",
            "snapshot-1",
            1_000,
            Some(expected),
        )
        .expect("expected-parent context");
        let proposal = TrustedReceiveProposal::new(
            context,
            vec![TrustedReceiveUpdate::new_with_old_object(
                "refs/heads/runtime",
                RefTransition::Update { fast_forward: true },
                vec![TrustedPathChange::Modification(String::from(
                    "sessions/run-1/message.json",
                ))],
                Some(String::from("2222222222222222222222222222222222222222")),
            )],
            512,
            1_024,
            2,
        );
        let mutation_called = Cell::new(false);
        let result = authorize_before_canonical_mutation(
            &Principal::runtime("runtime", "run-1", "snapshot-1"),
            Some(&CapabilityReceivePolicyGuard),
            Some(&proposal),
            |_| {
                mutation_called.set(true);
                Ok::<_, ()>(())
            },
        );

        assert!(matches!(
            result,
            Err(GuardedReceiveError::Policy(
                ReceivePolicyError::ExpectedParentMismatch
            ))
        ));
        assert!(!mutation_called.get());
    }

    #[test]
    fn runtime_receive_without_guard_fails_closed() {
        let mutation_called = Cell::new(false);
        let result = authorize_before_canonical_mutation(
            &Principal::runtime("runtime", "run-1", "snapshot-1"),
            None,
            Some(&proposal("sessions/run-1/message.json")),
            |_| {
                mutation_called.set(true);
                Ok::<_, ()>(())
            },
        );

        assert!(matches!(
            result,
            Err(GuardedReceiveError::Policy(
                ReceivePolicyError::RuntimeGuardUnavailable
            ))
        ));
        assert!(!mutation_called.get());
    }

    #[test]
    fn runtime_context_mismatch_fails_before_guard_and_mutation() {
        let mutation_called = Cell::new(false);
        let result = authorize_before_canonical_mutation(
            &Principal::runtime("runtime", "other-run", "snapshot-1"),
            Some(&CapabilityReceivePolicyGuard),
            Some(&proposal("sessions/run-1/message.json")),
            |_| {
                mutation_called.set(true);
                Ok::<_, ()>(())
            },
        );

        assert!(matches!(
            result,
            Err(GuardedReceiveError::Policy(
                ReceivePolicyError::RuntimeBindingMismatch
            ))
        ));
        assert!(!mutation_called.get());
    }

    #[test]
    fn existing_human_receive_behavior_is_unchanged_without_guard() {
        let identity = identity_domain::AuthenticatedIdentity::new(
            identity_domain::UserId::new(),
            "https://issuer.example",
            "user-1",
            serde_json::json!({}),
            identity_domain::RequestId::new(),
        );
        let result = authorize_before_canonical_mutation(
            &Principal::human(identity),
            None,
            None,
            |permit| {
                assert!(permit.is_none());
                Ok::<_, ()>("legacy-human-path")
            },
        );

        assert_eq!(result.expect("human mutation"), "legacy-human-path");
    }
}
