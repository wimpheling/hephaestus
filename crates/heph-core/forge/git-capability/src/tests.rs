use super::*;
use uuid::Uuid;

fn limits() -> TransferLimits {
    TransferLimits::new(1_024, 2_048, 100, 10).expect("valid limits")
}

fn receive_scope(
    refs: Vec<RefGlob>,
    paths: Vec<ChangedPathGlob>,
    policy: RefUpdatePolicy,
) -> GitCapabilityScope {
    GitCapabilityScope::new(GitCapabilityScopeInput {
        repository_id: RepositoryId::new(Uuid::from_u128(1)),
        operations: vec![GitOperation::Receive],
        ref_globs: refs,
        changed_path_globs: paths,
        update_policy: policy,
        expires_at_unix_seconds: 2_000_000_000,
        transfer_limits: limits(),
    })
    .expect("valid receive scope")
}

fn ceiling(
    refs: Vec<RefGlob>,
    paths: Vec<ChangedPathGlob>,
    update_policy: RefUpdatePolicy,
    transfer_limits: TransferLimits,
    exact_parent_required: bool,
) -> GitCapabilityCeiling {
    GitCapabilityCeiling::new(GitCapabilityCeilingInput {
        operations: vec![
            GitOperation::Discover,
            GitOperation::Fetch,
            GitOperation::Receive,
        ],
        ref_globs: refs,
        changed_path_globs: paths,
        update_policy,
        transfer_limits,
        exact_parent_required,
    })
    .expect("valid Git ceiling")
}

#[test]
fn instance_git_authority_can_only_attenuate_release_ceiling() {
    let permissive = RefUpdatePolicy {
        branches: BranchRefPolicy {
            updates: BranchUpdatePolicy::AllowForce,
            create: RefMutationPermission::Allow,
            delete: RefMutationPermission::Allow,
        },
        ..RefUpdatePolicy::default()
    };
    let release = ceiling(
        vec![
            RefGlob::parse("refs/heads/content").expect("content ref"),
            RefGlob::parse("refs/heads/drafts").expect("draft ref"),
        ],
        vec![
            ChangedPathGlob::parse("content/**").expect("content path"),
            ChangedPathGlob::parse("drafts/**").expect("draft path"),
        ],
        permissive,
        TransferLimits::new(4_096, 8_192, 200, 20).expect("release limits"),
        false,
    );
    let selected = ceiling(
        vec![RefGlob::parse("refs/heads/content").expect("content ref")],
        vec![ChangedPathGlob::parse("content/**").expect("content path")],
        RefUpdatePolicy::default(),
        limits(),
        true,
    );
    assert!(selected.is_attenuation_of(&release));
    let bound = BoundGitCapability::new(
        RepositoryId::new(Uuid::from_u128(1)),
        selected.clone(),
        &release,
    )
    .expect("narrow binding");
    assert_eq!(bound.authority(), &selected);

    let broader_limits = ceiling(
        vec![RefGlob::parse("refs/heads/content").expect("content ref")],
        vec![ChangedPathGlob::parse("content/**").expect("content path")],
        RefUpdatePolicy::default(),
        TransferLimits::new(8_192, 8_192, 200, 20).expect("broader limits"),
        false,
    );
    assert!(!broader_limits.is_attenuation_of(&release));
    assert!(
        BoundGitCapability::new(
            RepositoryId::new(Uuid::from_u128(1)),
            broader_limits,
            &release,
        )
        .is_err()
    );
}

#[test]
fn bound_hash_includes_exact_repository_identity() {
    let release = ceiling(
        vec![RefGlob::parse("refs/heads/content").expect("content ref")],
        vec![ChangedPathGlob::parse("content/**").expect("content path")],
        RefUpdatePolicy::default(),
        limits(),
        false,
    );
    let first = BoundGitCapability::new(
        RepositoryId::new(Uuid::from_u128(1)),
        release.clone(),
        &release,
    )
    .expect("first binding");
    let second = BoundGitCapability::new(
        RepositoryId::new(Uuid::from_u128(2)),
        release.clone(),
        &release,
    )
    .expect("second binding");
    assert_ne!(
        first.normalized_hash().expect("first hash"),
        second.normalized_hash().expect("second hash")
    );
}

#[test]
fn repository_id_requires_canonical_text() {
    let canonical = "00000000-0000-0000-0000-000000000001";
    assert_eq!(
        canonical
            .parse::<RepositoryId>()
            .expect("canonical")
            .to_string(),
        canonical
    );
    assert!(
        "00000000000000000000000000000001"
            .parse::<RepositoryId>()
            .is_err()
    );
    assert!(
        "00000000-0000-0000-0000-00000000000A"
            .parse::<RepositoryId>()
            .is_err()
    );
}

#[test]
fn strict_globs_reject_ambiguous_and_implicit_broad_forms() {
    for invalid in [
        "main",
        "refs/*/main",
        "refs/heads/a/**b",
        "refs/heads/a//b",
        "refs/heads/../main",
        "refs/heads/bad.lock",
        "refs/heads/a\\b",
    ] {
        assert!(RefGlob::parse(invalid).is_err(), "accepted {invalid:?}");
    }
    assert!(RefGlob::parse("refs/heads/**").is_err());
    assert!(RefGlob::parse("refs/heads/*").is_err());
    assert!(RefGlob::parse_explicitly_broad("refs/heads/**").is_ok());
    assert!(ChangedPathGlob::parse("**").is_err());
    assert!(ChangedPathGlob::parse("*").is_err());
    assert!(ChangedPathGlob::parse_explicitly_broad("**").is_ok());
}

#[test]
fn glob_matching_is_anchored_segmented_and_case_sensitive() {
    let refs = RefGlob::parse("refs/heads/release/*").expect("valid ref glob");
    assert!(refs.is_match("refs/heads/release/v1"));
    assert!(!refs.is_match("refs/heads/release/v1/patch"));
    assert!(!refs.is_match("refs/heads/Release/v1"));

    let paths =
        ChangedPathGlob::parse("sessions/**/message-*.json").expect("valid changed-path glob");
    assert!(paths.is_match("sessions/message-1.json"));
    assert!(paths.is_match("sessions/alice/2026/message-1.json"));
    assert!(!paths.is_match("archive/sessions/message-1.json"));
}

#[test]
fn unicode_matching_is_exact_without_normalization_or_case_folding() {
    let glob = ChangedPathGlob::parse("résumés/café.md").expect("valid Unicode glob");
    assert!(glob.is_match("résumés/café.md"));
    assert!(!glob.is_match("résumés/cafe\u{301}.md"));
    assert!(!glob.is_match("RÉSUMÉS/café.md"));
}

#[test]
fn normalization_sorts_deduplicates_and_hashes_stably() {
    let first = GitCapabilityScope::new(GitCapabilityScopeInput {
        repository_id: RepositoryId::new(Uuid::from_u128(1)),
        operations: vec![
            GitOperation::Fetch,
            GitOperation::Discover,
            GitOperation::Fetch,
        ],
        ref_globs: vec![
            RefGlob::parse("refs/tags/v*").expect("tag glob"),
            RefGlob::parse("refs/heads/main").expect("branch glob"),
            RefGlob::parse("refs/tags/v*").expect("tag glob"),
        ],
        changed_path_globs: Vec::new(),
        update_policy: RefUpdatePolicy::default(),
        expires_at_unix_seconds: 2_000_000_000,
        transfer_limits: limits(),
    })
    .expect("valid scope");
    let second = GitCapabilityScope::new(GitCapabilityScopeInput {
        repository_id: RepositoryId::new(Uuid::from_u128(1)),
        operations: vec![GitOperation::Discover, GitOperation::Fetch],
        ref_globs: vec![
            RefGlob::parse("refs/heads/main").expect("branch glob"),
            RefGlob::parse("refs/tags/v*").expect("tag glob"),
        ],
        changed_path_globs: Vec::new(),
        update_policy: RefUpdatePolicy::default(),
        expires_at_unix_seconds: 2_000_000_000,
        transfer_limits: limits(),
    })
    .expect("valid scope");

    assert_eq!(first, second);
    assert_eq!(
        first.normalized_hash().expect("hash"),
        second.normalized_hash().expect("hash")
    );
    assert_eq!(
        first.canonical_json().expect("JSON"),
        second.canonical_json().expect("JSON")
    );
}

#[test]
fn receive_scope_requires_paths_and_rejects_read_only_update_policy() {
    let common = |operations, paths, policy| {
        GitCapabilityScope::new(GitCapabilityScopeInput {
            repository_id: RepositoryId::new(Uuid::from_u128(1)),
            operations,
            ref_globs: vec![RefGlob::parse("refs/heads/main").expect("ref glob")],
            changed_path_globs: paths,
            update_policy: policy,
            expires_at_unix_seconds: 2_000_000_000,
            transfer_limits: limits(),
        })
    };
    assert!(
        common(
            vec![GitOperation::Receive],
            Vec::new(),
            RefUpdatePolicy::default()
        )
        .is_err()
    );
    let non_default = RefUpdatePolicy {
        branches: BranchRefPolicy {
            create: RefMutationPermission::Allow,
            ..BranchRefPolicy::default()
        },
        ..RefUpdatePolicy::default()
    };
    assert!(common(vec![GitOperation::Fetch], Vec::new(), non_default).is_err());
}

#[test]
fn receive_checks_transitions_and_every_rename_endpoint() {
    let policy = RefUpdatePolicy {
        branches: BranchRefPolicy {
            create: RefMutationPermission::Allow,
            ..BranchRefPolicy::default()
        },
        ..RefUpdatePolicy::default()
    };
    let scope = receive_scope(
        vec![RefGlob::parse("refs/heads/session-*").expect("ref glob")],
        vec![ChangedPathGlob::parse("sessions/**").expect("path glob")],
        policy,
    );
    let allowed = [PathChange::Addition("sessions/a/1.json")];
    assert!(scope.allows_receive(&ReceiveUpdate {
        reference: "refs/heads/session-a",
        transition: RefTransition::Create,
        changed_paths: &allowed,
    }));

    let escaped_rename = [PathChange::Rename {
        from: "sessions/a/1.json",
        to: "private/1.json",
    }];
    assert!(!scope.allows_receive(&ReceiveUpdate {
        reference: "refs/heads/session-a",
        transition: RefTransition::Update { fast_forward: true },
        changed_paths: &escaped_rename,
    }));
    assert!(!scope.allows_receive(&ReceiveUpdate {
        reference: "refs/heads/session-a",
        transition: RefTransition::Update {
            fast_forward: false,
        },
        changed_paths: &allowed,
    }));
}

#[test]
fn tag_and_deletion_rules_are_independent() {
    let policy = RefUpdatePolicy {
        tags: RefNamespacePolicy {
            create: RefMutationPermission::Allow,
            ..RefNamespacePolicy::default()
        },
        ..RefUpdatePolicy::default()
    };
    let scope = receive_scope(
        vec![RefGlob::parse("refs/tags/release-*").expect("ref glob")],
        vec![ChangedPathGlob::parse("release/**").expect("path glob")],
        policy,
    );
    let changes = [PathChange::Addition("release/manifest.json")];
    assert!(scope.allows_receive(&ReceiveUpdate {
        reference: "refs/tags/release-1",
        transition: RefTransition::Create,
        changed_paths: &changes,
    }));
    assert!(!scope.allows_receive(&ReceiveUpdate {
        reference: "refs/tags/release-1",
        transition: RefTransition::Delete,
        changed_paths: &changes,
    }));
}

#[path = "property_tests.rs"]
mod property_tests;
