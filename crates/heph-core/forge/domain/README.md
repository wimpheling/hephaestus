# Purpose

`forge-domain` gives source control and project workflows stable values that
can cross service boundaries safely. It represents projects and repositories,
fully qualified Git refs, complete commit IDs, accepted ref updates, and the
runtime session that authenticated a receive.

# Responsibilities

`GitRef::parse` accepts only valid fully qualified refs beneath `refs/`, while
`CommitSha::parse` accepts complete lowercase SHA-1 or SHA-256 IDs. These
values make a receive's old and new state unambiguous and let downstream work
refer to the exact commit rather than a mutable branch name.

Project and repository values carry ownership, default branch, publication
visibility, and run-trigger settings. `RuntimeReceiveProvenance` preserves the
exact runtime authority session resolved by the Git transport, giving receive
processing an auditable authenticated origin.

# When

Use this crate at repository creation and Git receive boundaries. Parse the
incoming ref and commit first, then put the validated values into `RefUpdate`
and the repository or receive command passed to Forge services.
