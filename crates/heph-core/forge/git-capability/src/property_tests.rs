use super::*;
use proptest::prelude::*;

#[test]
fn expiry_is_exclusive_and_transfer_limits_are_bounded() {
    assert!(TransferLimits::new(0, 1, 1, 1).is_err());
    assert!(TransferLimits::new(1, MAX_PACK_BYTES + 1, 1, 1).is_err());
    let scope = GitCapabilityScope::new(GitCapabilityScopeInput {
        repository_id: RepositoryId::new(Uuid::from_u128(1)),
        operations: vec![GitOperation::Fetch],
        ref_globs: vec![RefGlob::parse("refs/heads/main").expect("ref glob")],
        changed_path_globs: Vec::new(),
        update_policy: RefUpdatePolicy::default(),
        expires_at_unix_seconds: 100,
        transfer_limits: limits(),
    })
    .expect("scope");
    assert!(scope.is_active_at(99));
    assert!(!scope.is_active_at(100));
}

proptest! {
    #[test]
    fn normalization_is_permutation_and_duplicate_invariant(
        operation_indexes in prop::collection::vec(0_u8..2, 1..40),
        ref_indexes in prop::collection::vec(0_u8..4, 1..40),
    ) {
        let operation_for = |index| match index {
            0 => GitOperation::Discover,
            _ => GitOperation::Fetch,
        };
        let ref_for = |index| match index {
            0 => RefGlob::parse("refs/heads/main").expect("glob"),
            1 => RefGlob::parse("refs/heads/release-*").expect("glob"),
            2 => RefGlob::parse("refs/tags/v*").expect("glob"),
            _ => RefGlob::parse("refs/notes/build-*").expect("glob"),
        };
        let build = |operations: Vec<_>, refs: Vec<_>| {
            GitCapabilityScope::new(GitCapabilityScopeInput {
                repository_id: RepositoryId::new(Uuid::from_u128(42)),
                operations,
                ref_globs: refs,
                changed_path_globs: Vec::new(),
                update_policy: RefUpdatePolicy::default(),
                expires_at_unix_seconds: 2_000_000_000,
                transfer_limits: limits(),
            }).expect("scope")
        };
        let mut operations: Vec<_> = operation_indexes.into_iter().map(operation_for).collect();
        let mut refs: Vec<_> = ref_indexes.into_iter().map(ref_for).collect();
        let first = build(operations.clone(), refs.clone());
        operations.reverse();
        refs.reverse();
        operations.extend(operations.clone());
        refs.extend(refs.clone());
        let second = build(operations, refs);
        prop_assert_eq!(first.normalized_hash().expect("hash"), second.normalized_hash().expect("hash"));
    }

    #[test]
    fn literal_path_globs_match_only_identical_unicode_scalar_text(
        segments in prop::collection::vec("[a-zA-Z0-9é]{1,12}", 1..6),
    ) {
        let path = segments.join("/");
        let glob = ChangedPathGlob::parse(path.clone()).expect("literal glob");
        let prefixed = format!("prefix/{path}");
        prop_assert!(glob.is_match(&path));
        prop_assert!(!glob.is_match(&prefixed));
    }

    #[test]
    fn star_never_crosses_a_path_segment(
        left in "[a-z]{1,12}",
        right in "[a-z]{1,12}",
    ) {
        let glob = ChangedPathGlob::parse("root/*").expect("glob");
        let one_segment = format!("root/{left}");
        let two_segments = format!("root/{left}/{right}");
        prop_assert!(glob.is_match(&one_segment));
        prop_assert!(!glob.is_match(&two_segments));
    }
}
