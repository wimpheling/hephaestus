# MVP 01.3: Runtime Git scope and credential bridge

Owner: unassigned

## Outcome

Complete the authority bridge required for an agent runtime to use normal Git
against an exact repository without gaining broad repository access. Persist,
bind, snapshot, and authenticate the complete immutable Git scope that the
existing quarantined pre-receive enforcement path needs.

This task does not implement application Git behavior. It turns a declared,
bounded Git capability into an exact-run Git credential and a host-only
receive-policy context, so Git can atomically accept or reject a runtime push
before canonical refs change.

```text
release Git ceiling
  → exact instance-revision Git binding
  → immutable dispatch snapshot and expected parent
  → exact-run Git credential
  → Git HTTP runtime authentication
  → quarantined pre-receive validation
  → atomic receive or denial
```

## Why this is separate

MVP 01.1 now provides the strict Git grammar, PAT transport, and a quarantined
pre-receive hook. MVP 01.2 provides explicit publication modes and a named
repository slot for `runtime_git`. The persisted generic capability binding
currently records only a repository UUID and generic operations. It does not
record ref/path rules, transition policy, transfer limits, or the exact allowed
parent, so no production component can construct a `GitCapabilityScope` for a
runtime without broadening authority.

Failing closed is correct until this bridge exists.

## Locked decisions

| Area | Decision |
| --- | --- |
| Scope ownership | A Git scope is a typed extension of a repository capability binding, not a free-form runtime parameter, remote URL, or agent-supplied policy. |
| Ceiling and attenuation | A release declares the maximum Git scope. Instance setup may bind only one exact repository and a scope equal to or narrower than that ceiling. Dispatch may further narrow it, never broaden it. |
| Publication binding | `publication.repository_slot` names the required repository capability slot for `runtime_git`; its exact immutable revision binding is independent of trigger attachment routing. |
| Expected parent | A runtime-Git binding may require an exact old commit for a ref update. The dispatch snapshot records that parent when the release contract requests trigger-safe writes. It is not inferred from a later client push. |
| Credential | A runtime Git credential is opaque, short-lived, hash-only at rest, non-renewable, and bound to one runtime session, snapshot, repository binding, Git scope hash, and expiry. It is not a PAT or a user identity. |
| Transport | Git HTTP authenticates the runtime credential as a runtime principal. It resolves only the persisted immutable scope through a host-side context handle; bearer material and scope details never enter Git remotes, repository configuration, logs, or the pre-receive environment. |
| Receive enforcement | The pre-receive hook derives trusted pending facts from Git quarantine and checks the complete batch against the immutable scope. Missing scope/context, stale session, expired credential, or untrusted facts deny the entire receive. |
| Read boundary | Raw Git reads are limited by exact repository and visible refs. Changed-path restrictions apply only to receive; they do not claim to hide paths from clone/fetch. |

## Dependencies

- [`mvp-01-agent-principals-capabilities-and-runtime-authority.md`](mvp-01-agent-principals-capabilities-and-runtime-authority.md)
- [`mvp-01.1-git-capabilities-and-developer-personal-access-tokens.md`](mvp-01.1-git-capabilities-and-developer-personal-access-tokens.md)
- [`mvp-01.2-replace-controlled-result-publication-with-runtime-git.md`](mvp-01.2-replace-controlled-result-publication-with-runtime-git.md)

## Implementation checklist

- [x] **1. Persist typed Git capability ceilings**
  - [x] Extend release configuration so a repository capability slot may
    declare normalized ref globs, changed-path globs, receive transition rules,
    transfer limits, and whether an exact triggering parent is required.
  - [x] Reject remote URLs, repository names, bearer values, tenant IDs,
    ambiguous/broad globs without explicit opt-in, and Git policy outside the
    slot's repository resource kind.
  - [x] Store a versioned normalized Git-scope hash with the immutable release
    requirement and prove release publication preserves it.

- [x] **2. Bind and snapshot exact Git authority**
  - [x] Extend immutable instance-revision bindings with the selected narrower
    Git scope and validate it is an attenuation of the release ceiling.
  - [x] Bind `publication.repository_slot` to its exact repository capability
    binding; never infer it from an attachment or trigger repository.
  - [x] At dispatch, persist the complete scope hash, exact repository binding,
    optional expected parent, authorization snapshot, and expiry in the
    runtime-session records.
  - [x] Add PostgreSQL constraints and real integration tests for cross-project,
    scope broadening, stale revision, changed parent, missing binding, and
    historical snapshot retention.

- [x] **3. Issue and authenticate runtime Git credentials**
  - [x] Mint a separate opaque runtime Git credential only after the generic
    runtime session and Git scope snapshot are durable; store only its verifier.
  - [x] Deliver it through the existing sensitive runtime-authority bootstrap
    and exact acknowledgement lifecycle, never through a URL, environment,
    Git config, command line, or durable run record.
  - [x] Add Git HTTP authentication that resolves the runtime session, scope,
    expiry, live authorization, and revocation state before allowing discover,
    fetch, or receive.
  - [x] Prove replay denial across runs, revisions, bindings, repositories,
    operations, refs, and expired/revoked sessions.

- [x] **4. Complete scoped Git transport enforcement**
  - [x] Install the runtime principal's resolved scope in the host-only
    pre-receive context and use the quarantined hook for every runtime receive.
  - [x] Filter or deny ref advertisement and fetch outside the scope's exact
    repository/ref visibility boundary.
  - [x] Require the quarantined hook to validate every ref update, ancestry,
    changed path, rename endpoint, merge-parent path union, object/pack limit,
    expected parent, and transition rule before Git atomically updates refs.
  - [x] Record redacted allow/deny audit records with runtime session, snapshot,
    binding, scope hash, ref outcome, and reason code.

- [ ] **5. Prove recovery and handoff**
  - [ ] Define behavior for guest exit before acknowledgement, accepted push
    followed by guest crash, duplicate retry, receive failure, revocation during
    transfer, hook failure, and scope/parent conflict.
  - [ ] Treat an accepted receive as durable publication; retries with the same
    accepted ref transition are idempotent, while an ordinary non-fast-forward
    retry is denied without host-side rollback claims.
  - [ ] Preserve proposal-mode behavior and prove proposal runtimes cannot
    obtain a runtime Git credential or Git write remote.

- [ ] **6. Verify and document**
  - [ ] Document the ceiling/attenuation model, raw-read boundary, credential
    lifecycle, host-only context, quarantine trust boundary, expected-parent
    semantics, recovery, and residual authorized-destination risks.
  - [ ] Run `cargo fmt --all -- --check`,
    `cargo clippy --workspace --all-targets --all-features`,
    `cargo test --workspace --all-features`, and
    `cargo doc --workspace --all-features --no-deps`.
  - [ ] Run real PostgreSQL, real smart-HTTP, quarantined receive, runtime VM,
    revocation-race, credential-sentinel, and failure-injection scenarios.
  - [ ] Run `git diff --check` and `cargo dev quality` before handoff.

## Completion evidence

Record release requirement, revision binding, snapshot, runtime-session, and
scope-hash fixture IDs; allowed and denied smart-HTTP transcripts; quarantined
hook evidence; expected-parent and replay-denial cases; revocation timing;
credential-sentinel scans; and all verification command results or explicitly
justified environment exclusions.

## Non-goals

This task does not add a universal source-control policy engine, filtered
content reads, user PAT management, arbitrary Git hosting, or agent-defined
receive hooks. It does not implement the Git-native runtime worktree itself;
that remains MVP 01.2 once this authority bridge is available.
