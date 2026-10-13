//! Pure model/shape denials; no database, producer or native positive claims.

#[path = "tests/fixture.rs"]
mod fixture;

use super::{generic_error, graph, scope};
use runtime_authority::RuntimeAuthorityError;
use runtime_git_authority::RuntimeGitAuthorityError;
use uuid::Uuid;

#[test]
fn orphan_and_ambiguous_cardinalities_deny_instead_of_becoming_absence() {
    for counts in [(0, 1), (1, 0), (2, 0), (0, 2), (2, 1), (1, 2), (2, 2)] {
        assert_eq!(
            graph::shape(counts.0, counts.1),
            Err(RuntimeGitAuthorityError::IdentityMismatch)
        );
    }
}

#[test]
fn absence_rejects_every_original_binding_ceiling_or_operation_residue() {
    for (counts, declared, copied) in [
        ((1, 0), false, false),
        ((0, 1), false, false),
        ((0, 0), true, false),
        ((0, 0), false, true),
        ((-1, 0), false, false),
    ] {
        assert_eq!(
            graph::qualify_absence(counts, declared, copied),
            Err(RuntimeGitAuthorityError::IdentityMismatch)
        );
    }
}

#[test]
fn decode_denies_unknown_grammar_policy_limits_operations_and_raw_order() {
    // Pure decoder control only; no persisted source graph or authority proof.
    assert!(fixture::shape().decode().is_ok());
    let mut shape = fixture::shape();
    shape.grammar_version += 1;
    assert!(shape.decode().is_err());
    let mut shape = fixture::shape();
    shape.branch_update_policy = "unknown".into();
    assert!(shape.decode().is_err());
    let mut shape = fixture::shape();
    shape.request_bytes = -1;
    assert!(shape.decode().is_err());
    let mut shape = fixture::shape();
    shape.git_operations.push("unknown".into());
    assert!(shape.decode().is_err());
    let mut shape = fixture::shape();
    shape.git_operations.swap(0, 1);
    assert!(shape.decode().is_err());
    let mut shape = fixture::shape();
    shape.git_operations.push("fetch".into());
    assert!(shape.decode().is_err());
}

#[test]
fn original_generic_permission_ceiling_cannot_be_broadened() {
    let shape = fixture::shape();
    assert!(shape.generic_permissions(&[]).is_err());
    let mut shape = fixture::shape();
    shape.branch_create = true;
    assert!(shape.generic_permissions(&["git_read".into()]).is_err());
    let mut shape = fixture::shape();
    shape.tag_update = true;
    assert!(shape.generic_permissions(&["git_read".into()]).is_err());
    let mut shape = fixture::shape();
    shape.other_delete = true;
    assert!(shape.generic_permissions(&["git_read".into()]).is_err());
}

#[test]
fn every_original_link_hash_and_copied_shape_substitution_denies() {
    // Pure coherent model control, never worker/producer qualification evidence.
    let (original, snapshot, binding, release) = fixture::inputs();
    assert!(scope::qualify(&snapshot, &binding, &release, &original).is_ok());
    let (original, mut snapshot, binding, release) = fixture::inputs();
    snapshot.repository_id = Uuid::from_u128(99);
    assert!(scope::qualify(&snapshot, &binding, &release, &original).is_err());
    let (original, snapshot, mut binding, release) = fixture::inputs();
    binding.requirement_id = Uuid::from_u128(99);
    assert!(scope::qualify(&snapshot, &binding, &release, &original).is_err());
    let (original, snapshot, binding, mut release) = fixture::inputs();
    release.release_agent_id = Uuid::from_u128(99);
    assert!(scope::qualify(&snapshot, &binding, &release, &original).is_err());
    let (original, snapshot, binding, mut release) = fixture::inputs();
    release.requirement_hash[0] ^= 1;
    assert!(scope::qualify(&snapshot, &binding, &release, &original).is_err());
    let (original, mut snapshot, mut binding, release) = fixture::inputs();
    snapshot.shape.normalized_hash[0] ^= 1;
    binding.shape = snapshot.shape.clone();
    assert!(scope::qualify(&snapshot, &binding, &release, &original).is_err());
    let (original, mut snapshot, binding, release) = fixture::inputs();
    snapshot.shape.ref_globs.push("refs/heads/other".into());
    assert!(scope::qualify(&snapshot, &binding, &release, &original).is_err());
    let (original, mut snapshot, binding, release) = fixture::inputs();
    snapshot.expected_parent = Some("a".repeat(40));
    assert!(scope::qualify(&snapshot, &binding, &release, &original).is_err());
}

#[test]
fn generic_unsupported_missing_and_identity_errors_do_not_fallback() {
    assert_eq!(
        generic_error(RuntimeAuthorityError::Unsupported),
        RuntimeGitAuthorityError::Unsupported
    );
    assert_eq!(
        generic_error(RuntimeAuthorityError::NotFound),
        RuntimeGitAuthorityError::NotFound
    );
    assert_eq!(
        generic_error(RuntimeAuthorityError::IdentityMismatch),
        RuntimeGitAuthorityError::IdentityMismatch
    );
    assert_eq!(
        generic_error(RuntimeAuthorityError::GenerationMismatch),
        RuntimeGitAuthorityError::IdentityMismatch
    );
}
