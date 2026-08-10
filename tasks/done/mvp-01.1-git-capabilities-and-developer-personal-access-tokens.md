# MVP 01.1: Git capabilities and developer personal access tokens

Owner: unassigned

## Outcome

Provide the Git credential and enforcement layer required by MVP 01: a
production-ready non-interactive credential for developer Git and automation,
and a short-lived exact-run credential for an authorized agent runtime. Both
use one strict Git capability grammar while remaining distinct credential
types and principals.

## Locked decisions

- Browser sign-in remains OIDC-based.
- A PAT represents one user delegation; it is never issued to an agent run.
- A runtime Git capability authenticates one exact runtime session and is
  short-lived, opaque, non-renewable, and scope-bound to the exact run,
  repository binding, operations, refs, and write paths.
- Store only a one-way token verifier, never a recoverable token value.
- Every Git request, whether authenticated by OIDC, PAT, or runtime
  capability, must perform the applicable live authorization and scope check.
- Raw Git reads are repository/ref scoped. Path restrictions apply to writes
  at receive time; they do not conceal paths from clone or fetch.

## Dependencies

- [`mvp-01-agent-principals-capabilities-and-runtime-authority.md`](mvp-01-agent-principals-capabilities-and-runtime-authority.md)

## Implementation checklist

- [x] **1. Define one Git capability grammar**
  - [x] Define normalized repository IDs, Git operation scopes, ref globs,
    changed-path globs, fast-forward/force-update rules, tag/ref creation and
    deletion rules, expiry, and bounded transfer limits.
  - [x] Define deterministic matching semantics, including case, Unicode,
    `**`, ref namespaces, empty matches, additions, deletions, renames,
    merges, and new branches.
  - [x] Reject ambiguous, broad, malformed, or conflicting scopes by default.
  - [x] Add exhaustive unit and property tests for scope normalization and
    matching.

- [ ] **2. Add developer personal access tokens**
  - [x] Define a versioned opaque PAT format, token identifier, hash/verifier,
    user ownership, explicit scopes, optional repository restrictions, expiry,
    created/revoked/last-used timestamps, and audit metadata.
  - [ ] Add authenticated UI and API flows to create, list safe metadata for,
    rotate, and revoke PATs. Show the plaintext value exactly once at creation.
  - [ ] Add a local Git credential helper or CLI login flow that stores and
    refreshes credentials outside Git remote URLs and repository configuration.
  - [ ] Define token limits: bounded lifetime, scope minimization, rate limits,
    and immediate revocation behavior.

- [ ] **3. Add exact-run Git capabilities**
  - [ ] Materialize an opaque Git capability only from the MVP 01 runtime
    session and immutable repository binding; store only its verifier.
  - [ ] Bind it to the exact run, instance, revision, repository, declared
    operation set, ref/path scopes, issue/expiry time, and authorization
    snapshot.
  - [ ] Deliver it only through trusted runtime bootstrap and remove it when
    the run ends, is cancelled, expires, or loses live authority.
  - [ ] Prove that a capability cannot be replayed across runs, instances,
    revisions, repositories, refs, or operation classes.

- [ ] **4. Enforce capabilities in Git HTTP**
  - [ ] Extend Git HTTP authentication to accept PAT and runtime capability
    credentials without weakening OIDC JWT authentication.
  - [ ] Filter or deny ref discovery and fetch outside the allowed repository
    and ref scope.
  - [ ] Before a receive updates any ref, validate every operation, ref glob,
    transition rule, and changed path using trusted repository state.
  - [ ] Reject unauthorized object transfer safely, bound quarantine/storage
    use, preserve atomic receive behavior, and audit allowed and denied calls.
  - [ ] Add real Git smart-HTTP tests for clone/fetch/push, path/ref denial,
    creation/deletion/force-push denial, expiry, rotation, revocation, and
    audit redaction.

## Non-goals

- Replacing browser OIDC sessions with PATs.
- Giving agent guests a user PAT, broad Git write access, or path-restricted
  raw Git reads.
- Reusing the local `/test/git-token` fixture outside development.

## Verify and document

- Document the capability grammar, matching rules, PAT lifecycle, runtime
  capability lifecycle, and the distinction between read and write scope.
- Run unit and property coverage for normalized scope matching, plus real Git
  smart-HTTP integration coverage for the allowed and denied cases in this
  plan. Run PostgreSQL-backed persistence, expiry, rotation, revocation, and
  audit-redaction coverage where those paths require the production adapter.
- Run `cargo fmt --all -- --check`,
  `cargo clippy --workspace --all-targets --all-features`,
  `cargo test --workspace --all-features`, and
  `cargo doc --workspace --all-features --no-deps`.
- Before repository handoff, run `git diff --check` and `cargo dev quality`.

## Completion evidence

The completed task records the capability grammar version; allowed and denied
scope fixtures; smart-HTTP clone, fetch, and receive results; PAT rotation and
revocation evidence; exact-run capability expiry and cross-run replay-denial
evidence; and audit records proving that token plaintext is absent. It also
records the commands above, their results, and any explicitly justified test
environment exclusions.
