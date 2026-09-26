//! Receive-policy integration tests across the public transport boundary.

use git_capability_domain::{
    BranchRefPolicy, BranchUpdatePolicy, ChangedPathGlob, GitCapabilityScope,
    GitCapabilityScopeInput, GitOperation, RefGlob, RefMutationPermission, RefNamespacePolicy,
    RefTransition, RefUpdatePolicy, RepositoryId, TransferLimits,
};
use git_http::{
    Principal,
    receive_policy::{
        CapabilityReceivePolicyGuard, GuardedReceiveError, ReceivePolicyError,
        ResolvedRuntimeReceiveContext, TrustedPathChange, TrustedReceiveProposal,
        TrustedReceiveUpdate, authorize_before_canonical_mutation,
    },
};
use std::{cell::Cell, sync::Arc};
use uuid::Uuid;

#[test]
fn denied_command_in_atomic_batch_prevents_canonical_mutation() {
    let scope = Arc::new(
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
    );
    let context = ResolvedRuntimeReceiveContext::new(scope, "run-1", "snapshot-1", 1_000)
        .expect("resolved context");
    let proposal = TrustedReceiveProposal::new(
        context,
        vec![
            TrustedReceiveUpdate::new(
                "refs/heads/runtime",
                RefTransition::Create,
                vec![TrustedPathChange::Addition(String::from(
                    "sessions/allowed.json",
                ))],
            ),
            TrustedReceiveUpdate::new(
                "refs/heads/runtime",
                RefTransition::Create,
                vec![TrustedPathChange::Addition(String::from(
                    "outside/denied.json",
                ))],
            ),
        ],
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
            ReceivePolicyError::ScopeDenied { update_index: 1 }
        ))
    ));
    assert!(!mutation_called.get());
}
