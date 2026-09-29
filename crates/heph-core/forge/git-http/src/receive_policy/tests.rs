use super::{
    CapabilityReceivePolicyGuard, GuardedReceiveError, ReceivePolicyError,
    ResolvedRuntimeReceiveContext, TrustedPathChange, TrustedReceiveProposal, TrustedReceiveUpdate,
    authorize_before_canonical_mutation,
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
    let result =
        authorize_before_canonical_mutation(&Principal::human(identity), None, None, |permit| {
            assert!(permit.is_none());
            Ok::<_, ()>("legacy-human-path")
        });

    assert_eq!(result.expect("human mutation"), "legacy-human-path");
}
