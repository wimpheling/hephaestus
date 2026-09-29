# Purpose

`review-git` publishes an approved review result into the canonical bare Git
repository. It is the trusted host adapter used after the review control plane
has authorized an approval and recorded the exact input and result commits.

# Responsibilities

The publisher resolves the repository through canonical forge storage, verifies
that the controlled result ref points at the recorded result commit, checks
that the result commit's parent is the exact input commit, and performs a Git
compare-and-swap update of the target ref. If the target has already advanced,
it returns a conflict; if it already equals the result, it reports idempotent
approval. This preserves review provenance and prevents an unrelated commit or
path from being published by a stale approval.

# When

Construct the publisher with a repository locator and give it to the review
service:

```rust
let publisher = GitReviewPublisher::new(repository_locator);
```

The review repository consumes `ApprovalDisposition::Approved` or
`Conflicted` and commits the matching control outcome and event.
